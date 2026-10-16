//! Launching applications and running (authorised) shell commands.
//!
//! The launcher itself performs no authorisation: callers (the runcommand and
//! deck plugins) only pass commands that the *local* user configured. Remote
//! devices can reference commands by key, never send command text.

use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::Serialize;
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;

const MAX_OUTPUT: usize = 64 * 1024;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandOutput {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
}

#[async_trait]
pub trait LauncherBackend: Send + Sync {
    async fn run_shell(&self, command: &str, timeout: Duration) -> Result<CommandOutput>;
    /// Open a file, folder or URL with the default handler.
    async fn open(&self, target: &str) -> Result<()>;
    /// Start an application by name / path / `.desktop` id.
    async fn launch_app(&self, app: &str, args: &[String]) -> Result<()>;
}

pub struct NativeLauncher;

fn shell(command: &str) -> Command {
    #[cfg(windows)]
    {
        let mut c = Command::new("cmd");
        c.args(["/C", command]);
        c
    }
    #[cfg(not(windows))]
    {
        let mut c = Command::new("sh");
        c.args(["-c", command]);
        c
    }
}

fn truncate(b: &[u8]) -> String {
    let s = String::from_utf8_lossy(&b[..b.len().min(MAX_OUTPUT)]);
    s.into_owned()
}

#[async_trait]
impl LauncherBackend for NativeLauncher {
    async fn run_shell(&self, command: &str, timeout: Duration) -> Result<CommandOutput> {
        let child = shell(command)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .context("cannot start shell")?;
        match tokio::time::timeout(timeout, child.wait_with_output()).await {
            Ok(out) => {
                let out = out?;
                Ok(CommandOutput {
                    code: out.status.code(),
                    stdout: truncate(&out.stdout),
                    stderr: truncate(&out.stderr),
                    timed_out: false,
                })
            }
            Err(_) => Ok(CommandOutput { code: None, stdout: String::new(), stderr: String::new(), timed_out: true }),
        }
    }

    async fn open(&self, target: &str) -> Result<()> {
        #[cfg(feature = "desktop")]
        {
            let t = target.to_owned();
            tokio::task::spawn_blocking(move || open::that_detached(t)).await??;
            Ok(())
        }
        #[cfg(not(feature = "desktop"))]
        {
            let _ = target;
            anyhow::bail!("open not supported")
        }
    }

    async fn launch_app(&self, app: &str, args: &[String]) -> Result<()> {
        let mut cmd;
        #[cfg(target_os = "macos")]
        {
            if app.ends_with(".app") || !app.contains('/') {
                cmd = Command::new("open");
                cmd.args(["-a", app]);
                if !args.is_empty() {
                    cmd.arg("--args").args(args);
                }
            } else {
                cmd = Command::new(app);
                cmd.args(args);
            }
        }
        #[cfg(target_os = "linux")]
        {
            if app.ends_with(".desktop") {
                cmd = Command::new("gtk-launch");
                cmd.arg(app.trim_end_matches(".desktop")).args(args);
            } else {
                cmd = Command::new(app);
                cmd.args(args);
            }
        }
        #[cfg(windows)]
        {
            cmd = Command::new("cmd");
            cmd.args(["/C", "start", "", app]).args(args);
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
        {
            cmd = Command::new(app);
            cmd.args(args);
        }
        cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        cmd.spawn().with_context(|| format!("cannot launch {app}"))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[tokio::test]
    async fn runs_and_times_out() {
        let l = NativeLauncher;
        let out = l.run_shell("echo hi", Duration::from_secs(5)).await.unwrap();
        assert_eq!(out.stdout.trim(), "hi");
        assert_eq!(out.code, Some(0));
        let out = l.run_shell("sleep 5", Duration::from_millis(100)).await.unwrap();
        assert!(out.timed_out);
    }
}
