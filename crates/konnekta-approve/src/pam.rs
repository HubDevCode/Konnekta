//! `/etc/pam.d/<service>` edits for `konnekta approve enable / disable`.
//!
//! The line goes before the first `auth` line, so the phone is asked first
//! and the password follows when the helper fails:
//!
//! ```text
//! auth sufficient pam_exec.so quiet stdout /usr/lib/konnekta/konnekta-approve
//! ```

use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

pub const PAM_DIR: &str = "/etc/pam.d";
pub const BACKUP_DIR: &str = "/etc/konnekta/pam-backup";
/// Where the helper is installed.
pub const HELPER: &str = "/usr/lib/konnekta/konnekta-approve";
/// Services the phone may approve. `sshd` and login screens never.
pub const SERVICES: &[&str] = &["sudo", "sudo-i", "polkit-1"];
const MARK: &str = "konnekta-approve";

pub fn line(helper: &Path) -> String {
    format!(
        "auth sufficient pam_exec.so quiet stdout {}",
        helper.display()
    )
}

/// `text` with the helper line added before the first `auth` line, or
/// `None` if it is already there.
pub fn add_line(text: &str, helper: &Path) -> Option<String> {
    if text
        .lines()
        .any(|l| l.contains(MARK) && !l.trim_start().starts_with('#'))
    {
        return None;
    }
    let mut out = String::with_capacity(text.len() + 100);
    let mut added = false;
    for l in text.split_inclusive('\n') {
        let t = l.trim_start();
        if !added && (t.starts_with("auth") || t.starts_with("-auth")) {
            out.push_str(&line(helper));
            out.push('\n');
            added = true;
        }
        out.push_str(l);
    }
    if !added {
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str(&line(helper));
        out.push('\n');
    }
    Some(out)
}

/// `text` without the helper lines, or `None` if there were none.
pub fn remove_lines(text: &str) -> Option<String> {
    let out: String = text
        .split_inclusive('\n')
        .filter(|l| !l.contains(MARK))
        .collect();
    (out.len() != text.len()).then_some(out)
}

/// Rewrite `/etc/pam.d/<service>` atomically with `f`, keeping a copy in
/// `/etc/konnekta/pam-backup`. Returns the backup, or `None` when `f`
/// changed nothing. Run as root.
pub fn edit(
    service: &str,
    f: impl FnOnce(&str) -> Option<String>,
) -> std::io::Result<Option<PathBuf>> {
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let path = Path::new(PAM_DIR).join(service);
    let text = std::fs::read_to_string(&path)?;
    let Some(new) = f(&text) else {
        return Ok(None);
    };
    let backup_dir = Path::new(BACKUP_DIR);
    std::fs::create_dir_all(backup_dir)?;
    std::fs::set_permissions(backup_dir, std::fs::Permissions::from_mode(0o755))?;
    let backup = backup_dir.join(format!("{service}.{}", crate::helper::now_secs()));
    std::fs::write(&backup, &text)?;
    let tmp = Path::new(PAM_DIR).join(format!(".{service}.konnekta.tmp"));
    let _ = std::fs::remove_file(&tmp);
    {
        let mut out = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o644)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&tmp)?;
        out.write_all(new.as_bytes())?;
        out.sync_all()?;
    }
    std::fs::rename(&tmp, &path)?;
    Ok(Some(backup))
}

/// Whether `/etc/pam.d/<service>` asks the phone.
pub fn enabled(service: &str) -> bool {
    std::fs::read_to_string(Path::new(PAM_DIR).join(service))
        .map(|t| add_line(&t, Path::new("/x")).is_none())
        .unwrap_or(false)
}

/// The helper runs as root: it must not be replaceable by the user. The
/// file and every folder up to `/` must be owned by root and not writable
/// by group or others.
pub fn helper_is_safe(helper: &Path) -> Result<(), &'static str> {
    if !helper.is_absolute() {
        return Err("the helper path must be absolute");
    }
    let m = std::fs::symlink_metadata(helper).map_err(|_| "helper not found")?;
    if !m.file_type().is_file() || m.uid() != 0 || m.mode() & 0o022 != 0 || m.mode() & 0o111 == 0 {
        return Err("the helper must be a root-owned executable that only root can change");
    }
    let mut p = helper.parent();
    while let Some(d) = p {
        let m = std::fs::symlink_metadata(d).map_err(|_| "helper folder missing")?;
        if !m.file_type().is_dir() || m.uid() != 0 || m.mode() & 0o022 != 0 {
            return Err(
                "every folder of the helper path must be owned by root and not writable by others",
            );
        }
        p = d.parent();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SUDO: &str = "#%PAM-1.0\nauth\t\tinclude\t\tsystem-auth\naccount\t\tinclude\t\tsystem-auth\nsession\t\tinclude\t\tsystem-auth\n";

    #[test]
    fn add_and_remove() {
        let h = Path::new("/usr/lib/konnekta/konnekta-approve");
        let added = add_line(SUDO, h).unwrap();
        assert_eq!(
            added,
            "#%PAM-1.0\nauth sufficient pam_exec.so quiet stdout /usr/lib/konnekta/konnekta-approve\nauth\t\tinclude\t\tsystem-auth\naccount\t\tinclude\t\tsystem-auth\nsession\t\tinclude\t\tsystem-auth\n"
        );
        assert_eq!(add_line(&added, h), None, "idempotent");
        assert_eq!(remove_lines(&added).as_deref(), Some(SUDO));
        assert_eq!(remove_lines(SUDO), None);
        // No auth line: appended.
        assert!(
            add_line("account include x", h)
                .unwrap()
                .ends_with("konnekta-approve\n")
        );
    }

    #[test]
    fn user_owned_helper_is_refused() {
        let t = tempfile::tempdir().unwrap();
        let h = t.path().join("konnekta-approve");
        std::fs::write(&h, b"#!/bin/sh\n").unwrap();
        assert!(helper_is_safe(&h).is_err());
        assert!(helper_is_safe(Path::new("relative/helper")).is_err());
    }
}
