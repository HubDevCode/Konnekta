//! Local IPC between the CLI and the daemon: newline-delimited JSON over a
//! Unix socket (mode 0600, inside the user's runtime dir) or a Windows named
//! pipe that rejects remote clients. Only the local user can control the
//! daemon; nothing is exposed on the network.

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::path::PathBuf;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};

pub fn endpoint(data_dir: &std::path::Path) -> PathBuf {
    #[cfg(unix)]
    {
        let base = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(|| data_dir.to_path_buf());
        base.join("konnekta.sock")
    }
    #[cfg(windows)]
    {
        let _ = data_dir;
        let user = std::env::var("USERNAME").unwrap_or_else(|_| "user".into());
        PathBuf::from(format!(r"\\.\pipe\konnekta-{user}"))
    }
}

/// Serve one client connection.
pub async fn serve_client<S>(stream: S, k: konnekta_core::Konnekta)
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (rd, mut wr) = tokio::io::split(stream);
    let mut lines = BufReader::new(rd).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let Ok(req) = serde_json::from_str::<Value>(&line) else { break };
        let id = req.get("id").cloned().unwrap_or(Value::Null);
        let method = req.get("method").and_then(Value::as_str).unwrap_or_default().to_owned();
        if method == "subscribe" {
            let mut rx = k.subscribe();
            loop {
                match rx.recv().await {
                    Ok(ev) => {
                        let mut out = serde_json::to_vec(&ev).unwrap_or_default();
                        out.push(b'\n');
                        if wr.write_all(&out).await.is_err() {
                            return;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(_) => return,
                }
            }
        }
        let params = req.get("params").cloned().unwrap_or(Value::Null);
        let resp = match konnekta_core::api::call(&k, &method, params).await {
            Ok(r) => json!({ "id": id, "result": r }),
            Err(e) => json!({ "id": id, "error": format!("{e:#}") }),
        };
        let mut out = serde_json::to_vec(&resp).unwrap_or_default();
        out.push(b'\n');
        if wr.write_all(&out).await.is_err() {
            break;
        }
    }
}

#[cfg(unix)]
pub async fn listen(path: PathBuf, k: konnekta_core::Konnekta) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    if path.exists() {
        // Refuse to steal the socket of a running daemon.
        if tokio::net::UnixStream::connect(&path).await.is_ok() {
            bail!("another konnekta daemon is already running ({})", path.display());
        }
        std::fs::remove_file(&path)?;
    }
    let listener = tokio::net::UnixListener::bind(&path)?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    tracing::info!("control socket: {}", path.display());
    loop {
        let (s, _) = listener.accept().await?;
        tokio::spawn(serve_client(s, k.clone()));
    }
}

#[cfg(windows)]
pub async fn listen(path: PathBuf, k: konnekta_core::Konnekta) -> Result<()> {
    use tokio::net::windows::named_pipe::ServerOptions;
    let name = path.to_string_lossy().into_owned();
    let mut server = ServerOptions::new()
        .first_pipe_instance(true)
        .reject_remote_clients(true)
        .create(&name)
        .context("another konnekta daemon is already running")?;
    tracing::info!("control pipe: {name}");
    loop {
        server.connect().await?;
        let client = server;
        server = ServerOptions::new().reject_remote_clients(true).create(&name)?;
        tokio::spawn(serve_client(client, k.clone()));
    }
}

pub struct Client {
    rd: tokio::io::Lines<BufReader<Box<dyn AsyncRead + Unpin + Send>>>,
    wr: Box<dyn AsyncWrite + Unpin + Send>,
    next: u64,
}

impl Client {
    pub async fn connect(path: &std::path::Path) -> Result<Self> {
        #[cfg(unix)]
        let s = tokio::net::UnixStream::connect(path).await;
        #[cfg(windows)]
        let s = tokio::net::windows::named_pipe::ClientOptions::new().open(path);
        let s = s.with_context(|| "cannot reach the Konnekta daemon — start it with `konnekta daemon`".to_string())?;
        let (rd, wr) = tokio::io::split(s);
        Ok(Self {
            rd: BufReader::new(Box::new(rd) as Box<dyn AsyncRead + Unpin + Send>).lines(),
            wr: Box::new(wr),
            next: 1,
        })
    }

    async fn send(&mut self, method: &str, params: Value) -> Result<()> {
        let mut out = serde_json::to_vec(&json!({ "id": self.next, "method": method, "params": params }))?;
        out.push(b'\n');
        self.next += 1;
        self.wr.write_all(&out).await?;
        Ok(())
    }

    pub async fn call(&mut self, method: &str, params: Value) -> Result<Value> {
        self.send(method, params).await?;
        let line = self.rd.next_line().await?.context("daemon closed the connection")?;
        let v: Value = serde_json::from_str(&line)?;
        if let Some(e) = v.get("error").and_then(Value::as_str) {
            bail!("{e}");
        }
        Ok(v.get("result").cloned().unwrap_or(Value::Null))
    }

    pub async fn subscribe(mut self, mut f: impl FnMut(Value)) -> Result<()> {
        self.send("subscribe", Value::Null).await?;
        while let Some(line) = self.rd.next_line().await? {
            if let Ok(v) = serde_json::from_str(&line) {
                f(v);
            }
        }
        Ok(())
    }
}
