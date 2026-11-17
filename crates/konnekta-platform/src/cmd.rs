//! Helpers to run helper programs (playerctl, wl-copy, osascript...).

use anyhow::{Context, Result, bail};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::AsyncWriteExt;

/// Whether `program` is installed (the AppImage's own folders do not count).
pub fn exists(program: &str) -> bool {
    let Some(path) = crate::host::path() else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| {
        let p = dir.join(program);
        p.is_file() || p.with_extension("exe").is_file()
    })
}

/// Run a program (no shell) and return trimmed stdout.
pub async fn output(program: &str, args: &[&str]) -> Result<String> {
    let out = tokio::time::timeout(
        Duration::from_secs(5),
        crate::host::command_async(program)
            .args(args)
            .stdin(Stdio::null())
            .kill_on_drop(true)
            .output(),
    )
    .await
    .with_context(|| format!("{program} timed out"))?
    .with_context(|| format!("failed to run {program}"))?;
    if !out.status.success() {
        bail!(
            "{program} failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .trim_end_matches(['\n', '\r'])
        .to_string())
}

/// Run a program feeding `input` on stdin.
pub async fn with_stdin(program: &str, args: &[&str], input: &[u8]) -> Result<()> {
    let mut child = crate::host::command_async(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("failed to run {program}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(input).await?;
    }
    // wl-copy forks to serve the selection; don't wait forever.
    let _ = tokio::time::timeout(Duration::from_secs(5), child.wait()).await;
    Ok(())
}
