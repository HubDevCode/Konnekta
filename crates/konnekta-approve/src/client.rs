//! Talk to the user's Konnekta daemon from the root helper.
//!
//! The socket path is derived from the uid only (`/run/user/<uid>/konnekta.sock`),
//! never from the environment. Before sending anything the helper checks
//! that the folder and the socket belong to the user and, with
//! `SO_PEERCRED`, that the process listening runs as that user. Each line
//! from the daemon is limited to 64 KiB.

use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const MAX_LINE: u64 = 64 * 1024;

pub fn socket_path(uid: u32) -> PathBuf {
    PathBuf::from(format!("/run/user/{uid}/konnekta.sock"))
}

#[derive(Debug)]
pub struct ClientError(pub String);

impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ClientError {}

fn err<T>(m: impl Into<String>) -> Result<T, ClientError> {
    Err(ClientError(m.into()))
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn peer_uid(s: &UnixStream) -> Option<u32> {
    use std::os::fd::AsRawFd;
    let mut cred = libc::ucred {
        pid: 0,
        uid: u32::MAX,
        gid: 0,
    };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: valid fd, out-pointer and length for SO_PEERCRED.
    let r = unsafe {
        libc::getsockopt(
            s.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&mut cred as *mut libc::ucred).cast(),
            &mut len,
        )
    };
    (r == 0).then_some(cred.uid)
}

#[cfg(not(any(target_os = "linux", target_os = "android")))]
fn peer_uid(s: &UnixStream) -> Option<u32> {
    use std::os::fd::AsRawFd;
    let (mut uid, mut gid) = (u32::MAX, u32::MAX);
    // SAFETY: valid fd and out-pointers.
    let r = unsafe { libc::getpeereid(s.as_raw_fd(), &mut uid, &mut gid) };
    (r == 0).then_some(uid)
}

pub struct Client {
    rd: BufReader<UnixStream>,
    wr: UnixStream,
    next: u64,
}

impl Client {
    /// Connect to `path` and check that the daemon runs as `uid`.
    pub fn connect(path: &Path, uid: u32, timeout: Duration) -> Result<Self, ClientError> {
        let dir = path.parent().ok_or(ClientError("bad socket path".into()))?;
        let dm =
            std::fs::symlink_metadata(dir).map_err(|_| ClientError("no runtime folder".into()))?;
        if !dm.file_type().is_dir() || dm.uid() != uid || dm.mode() & 0o022 != 0 {
            return err("runtime folder is not the user's");
        }
        let sm = std::fs::symlink_metadata(path)
            .map_err(|_| ClientError("Konnekta is not running".into()))?;
        if !sm.file_type().is_socket() || sm.uid() != uid {
            return err("socket is not the user's");
        }
        let s =
            UnixStream::connect(path).map_err(|_| ClientError("Konnekta is not running".into()))?;
        if peer_uid(&s) != Some(uid) {
            return err("daemon runs as another user");
        }
        s.set_read_timeout(Some(timeout)).ok();
        s.set_write_timeout(Some(timeout)).ok();
        let wr = s.try_clone().map_err(|e| ClientError(e.to_string()))?;
        Ok(Self {
            rd: BufReader::new(s),
            wr,
            next: 1,
        })
    }

    pub fn set_timeout(&self, t: Duration) {
        self.wr.set_read_timeout(Some(t)).ok();
    }

    /// One JSON call; the result or the daemon's error message.
    pub fn call(&mut self, method: &str, params: Value) -> Result<Value, ClientError> {
        let mut out =
            serde_json::to_vec(&json!({ "id": self.next, "method": method, "params": params }))
                .map_err(|e| ClientError(e.to_string()))?;
        out.push(b'\n');
        self.next += 1;
        self.wr
            .write_all(&out)
            .map_err(|_| ClientError("daemon closed the connection".into()))?;
        let mut line = Vec::new();
        let n = (&mut self.rd)
            .take(MAX_LINE)
            .read_until(b'\n', &mut line)
            .map_err(|_| ClientError("no answer from the daemon".into()))?;
        if n == 0 || line.last() != Some(&b'\n') {
            return err("daemon answer too long or cut");
        }
        let v: Value =
            serde_json::from_slice(&line).map_err(|_| ClientError("bad daemon answer".into()))?;
        if let Some(e) = v.get("error").and_then(Value::as_str) {
            return err(e.chars().take(200).collect::<String>());
        }
        Ok(v.get("result").cloned().unwrap_or(Value::Null))
    }

    /// An action of the daemon's `approve` plugin for the phone `device`.
    pub fn approve(
        &mut self,
        device: &str,
        action: &str,
        args: Value,
    ) -> Result<Value, ClientError> {
        self.call(
            "pluginAction",
            json!({ "deviceId": device, "plugin": "approve", "action": action, "args": args }),
        )
    }
}
