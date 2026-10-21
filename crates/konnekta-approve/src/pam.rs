//! `/etc/pam.d/<service>` edits for `konnekta approve enable / disable`.
//!
//! The line goes before the first `auth` line, so the phone is asked first
//! and the password follows when the helper fails:
//!
//! ```text
//! auth sufficient pam_exec.so quiet stdout /usr/lib/konnekta/konnekta-approve
//! ```

use std::os::unix::fs::MetadataExt;
use std::path::Path;

pub const PAM_DIR: &str = "/etc/pam.d";
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
