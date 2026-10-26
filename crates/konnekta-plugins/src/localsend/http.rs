//! The little HTTP/1.1 the LocalSend protocol needs: JSON requests and one
//! streamed body per upload. One request per connection (`Connection:
//! close`), bodies with `Content-Length` or chunked.

use anyhow::{Context, Result, bail};
use std::path::Path;
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

pub trait Io: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Io for T {}

pub type Stream = BufReader<Box<dyn Io>>;

const MAX_HEAD: usize = 16 * 1024;

/// Request or response head.
#[derive(Debug, Default)]
pub struct Head {
    pub method: String,
    /// Path and query of a request.
    pub target: String,
    /// Status of a response.
    pub status: u16,
    headers: Vec<(String, String)>,
}

impl Head {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    pub fn path(&self) -> &str {
        self.target.split('?').next().unwrap_or("")
    }

    pub fn query(&self, key: &str) -> Option<String> {
        let q = self.target.split_once('?')?.1;
        q.split('&').find_map(|kv| {
            let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
            (decode(k) == key).then(|| decode(v))
        })
    }

    fn length(&self) -> BodyLen {
        if self
            .header("transfer-encoding")
            .is_some_and(|t| t.to_ascii_lowercase().contains("chunked"))
        {
            return BodyLen::Chunked {
                left: 0,
                done: false,
            };
        }
        match self
            .header("content-length")
            .and_then(|l| l.trim().parse().ok())
        {
            Some(n) => BodyLen::Fixed(n),
            None => BodyLen::Fixed(0),
        }
    }

    /// Declared body size, if any.
    pub fn content_length(&self) -> Option<u64> {
        self.header("content-length")
            .and_then(|l| l.trim().parse().ok())
    }
}

fn decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < b.len() => {
                let hex = |c: u8| (c as char).to_digit(16);
                match (hex(b[i + 1]), hex(b[i + 2])) {
                    (Some(h), Some(l)) => {
                        out.push((h * 16 + l) as u8);
                        i += 2;
                    }
                    _ => out.push(b'%'),
                }
            }
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Percent-encode a query value.
pub fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            b => format!("%{b:02X}"),
        })
        .collect()
}

async fn read_lines(s: &mut Stream) -> Result<Option<Vec<String>>> {
    let mut lines = Vec::new();
    let mut total = 0;
    loop {
        let mut line = Vec::new();
        let n = (&mut *s)
            .take(MAX_HEAD as u64)
            .read_until(b'\n', &mut line)
            .await?;
        if n == 0 {
            if lines.is_empty() {
                return Ok(None);
            }
            bail!("connection closed in the headers");
        }
        total += n;
        if total > MAX_HEAD {
            bail!("headers too long");
        }
        let line = String::from_utf8_lossy(&line)
            .trim_end_matches(['\r', '\n'])
            .to_owned();
        if line.is_empty() {
            if lines.is_empty() {
                continue; // stray CRLF between requests
            }
            return Ok(Some(lines));
        }
        lines.push(line);
    }
}

fn headers(lines: &[String]) -> Vec<(String, String)> {
    lines
        .iter()
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned()))
        .collect()
}

/// Next request on a server connection; None when the client closed it.
pub async fn read_request(s: &mut Stream) -> Result<Option<Head>> {
    let Some(lines) = read_lines(s).await? else {
        return Ok(None);
    };
    let mut first = lines[0].split_whitespace();
    let method = first.next().context("bad request line")?.to_owned();
    let target = first.next().context("bad request line")?.to_owned();
    Ok(Some(Head {
        method,
        target,
        status: 0,
        headers: headers(&lines[1..]),
    }))
}

async fn read_response_head(s: &mut Stream) -> Result<Head> {
    let lines = read_lines(s).await?.context("no answer")?;
    let status = lines[0]
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse().ok())
        .context("bad status line")?;
    Ok(Head {
        status,
        headers: headers(&lines[1..]),
        ..Default::default()
    })
}

enum BodyLen {
    Fixed(u64),
    Chunked { left: u64, done: bool },
    UntilEof,
}

/// A message body read from the stream.
pub struct Body<'a> {
    s: &'a mut Stream,
    len: BodyLen,
}

impl<'a> Body<'a> {
    pub fn new(s: &'a mut Stream, head: &Head) -> Self {
        Self {
            s,
            len: head.length(),
        }
    }

    pub async fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        loop {
            match &mut self.len {
                BodyLen::Fixed(0) => return Ok(0),
                BodyLen::Fixed(left) => {
                    let want = buf.len().min(*left as usize);
                    let n = self.s.read(&mut buf[..want]).await?;
                    if n == 0 {
                        bail!("connection closed in the body");
                    }
                    *left -= n as u64;
                    return Ok(n);
                }
                BodyLen::UntilEof => return Ok(self.s.read(buf).await?),
                BodyLen::Chunked { done: true, .. } => return Ok(0),
                BodyLen::Chunked { left, done } => {
                    if *left == 0 {
                        let mut line = String::new();
                        (&mut *self.s).take(1024).read_line(&mut line).await?;
                        let size = line.trim().split(';').next().unwrap_or("");
                        if size.is_empty() {
                            continue; // CRLF closing the previous chunk
                        }
                        let n = u64::from_str_radix(size, 16).context("bad chunk size")?;
                        if n == 0 {
                            // Trailers end with an empty line.
                            loop {
                                let mut t = String::new();
                                let r = (&mut *self.s).take(1024).read_line(&mut t).await?;
                                if r == 0 || t.trim().is_empty() {
                                    break;
                                }
                            }
                            *done = true;
                            return Ok(0);
                        }
                        *left = n;
                    }
                    let want = buf.len().min(*left as usize);
                    let n = self.s.read(&mut buf[..want]).await?;
                    if n == 0 {
                        bail!("connection closed in the body");
                    }
                    *left -= n as u64;
                    return Ok(n);
                }
            }
        }
    }

    /// The whole body, refusing more than `limit` bytes.
    pub async fn to_vec(&mut self, limit: usize) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        let mut buf = [0u8; 8192];
        loop {
            let n = self.read(&mut buf).await?;
            if n == 0 {
                return Ok(out);
            }
            if out.len() + n > limit {
                bail!("body too large");
            }
            out.extend_from_slice(&buf[..n]);
        }
    }
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        409 => "Conflict",
        422 => "Unprocessable Entity",
        429 => "Too Many Requests",
        _ => "Internal Server Error",
    }
}

/// Answer and end the exchange (the server closes after every response).
pub async fn respond(s: &mut Stream, status: u16, json: Option<&serde_json::Value>) -> Result<()> {
    let body = json.map(|j| j.to_string()).unwrap_or_default();
    let mut head = format!(
        "HTTP/1.1 {status} {}\r\nConnection: close\r\n",
        reason(status)
    );
    if json.is_some() {
        head.push_str("Content-Type: application/json\r\n");
    }
    head.push_str(&format!("Content-Length: {}\r\n\r\n", body.len()));
    let w = s.get_mut();
    w.write_all(head.as_bytes()).await?;
    w.write_all(body.as_bytes()).await?;
    w.flush().await?;
    let _ = w.shutdown().await;
    Ok(())
}

/// Where a LocalSend device listens.
#[derive(Debug, Clone)]
pub struct Target {
    pub host: String,
    pub port: u16,
    pub https: bool,
}

fn tls() -> Result<tokio_rustls::TlsConnector> {
    static CFG: OnceLock<Arc<tokio_rustls::rustls::ClientConfig>> = OnceLock::new();
    let cfg = match CFG.get() {
        Some(c) => c.clone(),
        None => {
            let c = konnekta_crypto::open_client_config()?;
            CFG.get_or_init(|| c).clone()
        }
    };
    Ok(tokio_rustls::TlsConnector::from(cfg))
}

async fn connect(t: &Target, timeout: Duration) -> Result<Stream> {
    let addr = if t.host.contains(':') {
        format!("[{}]:{}", t.host, t.port)
    } else {
        format!("{}:{}", t.host, t.port)
    };
    let tcp = tokio::time::timeout(timeout, TcpStream::connect(&addr))
        .await
        .context("connection timed out")??;
    let _ = tcp.set_nodelay(true);
    let io: Box<dyn Io> = if t.https {
        let name = tokio_rustls::rustls::pki_types::ServerName::try_from(t.host.clone())
            .unwrap_or_else(|_| {
                tokio_rustls::rustls::pki_types::ServerName::try_from("localsend").unwrap()
            });
        let s = tokio::time::timeout(timeout, tls()?.connect(name, tcp))
            .await
            .context("TLS handshake timed out")??;
        Box::new(s)
    } else {
        Box::new(tcp)
    };
    Ok(BufReader::new(io))
}

fn request_head(t: &Target, method: &str, target: &str, ctype: Option<&str>, len: u64) -> String {
    let mut h = format!(
        "{method} {target} HTTP/1.1\r\nHost: {}:{}\r\nUser-Agent: Konnekta\r\nConnection: close\r\nContent-Length: {len}\r\n",
        t.host, t.port
    );
    if let Some(c) = ctype {
        h.push_str(&format!("Content-Type: {c}\r\n"));
    }
    h.push_str("\r\n");
    h
}

async fn read_answer(s: &mut Stream, limit: usize) -> Result<(u16, Vec<u8>)> {
    let head = read_response_head(s).await?;
    let mut body =
        if head.header("content-length").is_none() && head.header("transfer-encoding").is_none() {
            Body {
                s,
                len: BodyLen::UntilEof,
            }
        } else {
            Body::new(s, &head)
        };
    let data = body.to_vec(limit).await.unwrap_or_default();
    Ok((head.status, data))
}

/// One JSON (or empty) request; `wait` bounds the whole exchange, which
/// for `prepare-upload` includes the time the user takes to accept.
pub async fn request(
    t: &Target,
    method: &str,
    target: &str,
    json: Option<&serde_json::Value>,
    wait: Duration,
) -> Result<(u16, Vec<u8>)> {
    let body = json.map(|j| j.to_string()).unwrap_or_default();
    let mut s = connect(t, Duration::from_secs(5).min(wait)).await?;
    let head = request_head(
        t,
        method,
        target,
        json.map(|_| "application/json"),
        body.len() as u64,
    );
    s.get_mut().write_all(head.as_bytes()).await?;
    s.get_mut().write_all(body.as_bytes()).await?;
    s.get_mut().flush().await?;
    tokio::time::timeout(wait, read_answer(&mut s, 1 << 20))
        .await
        .context("no answer in time")?
}

/// Stream a file as the body of a POST; `progress` gets the bytes sent.
pub async fn upload(
    t: &Target,
    target: &str,
    path: &Path,
    mut progress: impl FnMut(u64) + Send,
) -> Result<(u16, Vec<u8>)> {
    let mut f = tokio::fs::File::open(path)
        .await
        .with_context(|| format!("cannot read {}", path.display()))?;
    let size = f.metadata().await?.len();
    let mut s = connect(t, Duration::from_secs(10)).await?;
    let head = request_head(t, "POST", target, Some("application/octet-stream"), size);
    s.get_mut().write_all(head.as_bytes()).await?;
    let mut buf = vec![0u8; 256 * 1024];
    let mut sent = 0u64;
    while sent < size {
        let n = f.read(&mut buf).await?;
        if n == 0 {
            bail!("{} got shorter while sending", path.display());
        }
        let n = n.min((size - sent) as usize);
        tokio::time::timeout(Duration::from_secs(60), s.get_mut().write_all(&buf[..n]))
            .await
            .context("the receiver stopped reading")??;
        sent += n as u64;
        progress(sent);
    }
    s.get_mut().flush().await?;
    tokio::time::timeout(Duration::from_secs(120), read_answer(&mut s, 1 << 16))
        .await
        .context("no answer in time")?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_decoding() {
        let h = Head {
            target: "/api/localsend/v2/upload?sessionId=ab%20c&fileId=x+y&token=t".into(),
            ..Default::default()
        };
        assert_eq!(h.path(), "/api/localsend/v2/upload");
        assert_eq!(h.query("sessionId").as_deref(), Some("ab c"));
        assert_eq!(h.query("fileId").as_deref(), Some("x y"));
        assert_eq!(h.query("pin"), None);
        assert_eq!(encode("a b/é"), "a%20b%2F%C3%A9");
    }

    #[tokio::test]
    async fn chunked_body() {
        let raw = b"POST /x HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n4\r\nWiki\r\n5\r\npedia\r\n0\r\n\r\n";
        let io: Box<dyn Io> = Box::new(tokio_test_io(raw));
        let mut s = BufReader::new(io);
        let head = read_request(&mut s).await.unwrap().unwrap();
        let body = Body::new(&mut s, &head).to_vec(100).await.unwrap();
        assert_eq!(body, b"Wikipedia");
    }

    fn tokio_test_io(data: &[u8]) -> tokio::io::DuplexStream {
        let (a, mut b) = tokio::io::duplex(4096);
        let data = data.to_vec();
        tokio::spawn(async move {
            b.write_all(&data).await.unwrap();
            // keep `b` open briefly so reads do not see EOF early
            tokio::time::sleep(Duration::from_millis(50)).await;
        });
        a
    }
}
