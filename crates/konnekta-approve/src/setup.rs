//! One-step setup for the desktop app: everything root has to do, run once
//! through `pkexec` (the system asks for the password in its own window).
//!
//! * [`install`]: copy the helper to [`pam::HELPER`], write the user's key
//!   file and let the phone approve `sudo`;
//! * [`remove`]: undo it (PAM lines first, then the key).
//!
//! The key arrives from the user's own app, which already checked the
//! phone's signature and had the user compare the code; the trust is the
//! same as `sudo konnekta approve enroll` typed in that session.

use crate::key::KeyFile;
use crate::{keyfile, pam};
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::Path;

/// PAM services turned on by the one-step setup. `sudo-i` only where the
/// distribution has it.
pub const SERVICES: &[&str] = &["sudo", "sudo-i"];

fn need_root() -> Result<(), String> {
    // SAFETY: geteuid never fails.
    if unsafe { libc::geteuid() } != 0 {
        return Err("serve l'accesso da amministratore".into());
    }
    Ok(())
}

/// Copy `src` to the fixed helper path: root-owned, 0755, atomic.
fn install_helper(src: &Path) -> Result<(), String> {
    let dst = Path::new(pam::HELPER);
    let dir = dst.parent().ok_or("percorso dell'helper non valido")?;
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o755))
        .map_err(|e| e.to_string())?;
    let bytes = std::fs::read(src).map_err(|e| format!("{}: {e}", src.display()))?;
    let tmp = dir.join(".konnekta-approve.tmp");
    let _ = std::fs::remove_file(&tmp);
    {
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o755)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&tmp)
            .map_err(|e| e.to_string())?;
        f.write_all(&bytes).map_err(|e| e.to_string())?;
        f.sync_all().map_err(|e| e.to_string())?;
    }
    std::fs::rename(&tmp, dst).map_err(|e| e.to_string())?;
    pam::helper_is_safe(dst).map_err(String::from)
}

/// Install the helper from `helper_src`, write `key_text` (a key file as
/// [`KeyFile::render`] makes it) for `user`, turn on [`SERVICES`].
pub fn install(user: &str, key_text: &str, helper_src: &Path) -> Result<(), String> {
    need_root()?;
    let key = KeyFile::parse(key_text).map_err(|e| e.to_string())?;
    install_helper(helper_src)?;
    keyfile::write(Path::new(keyfile::KEY_DIR), user, &key).map_err(|e| e.to_string())?;
    for s in SERVICES {
        if Path::new(pam::PAM_DIR).join(s).exists() {
            pam::edit(s, |t| pam::add_line(t, Path::new(pam::HELPER)))
                .map_err(|e| format!("/etc/pam.d/{s}: {e}"))?;
        }
    }
    Ok(())
}

/// Stop asking the phone (every service) and forget `user`'s key. The
/// helper stays: other users may still use it.
pub fn remove(user: &str) -> Result<(), String> {
    need_root()?;
    for s in pam::SERVICES {
        if Path::new(pam::PAM_DIR).join(s).exists() {
            pam::edit(s, pam::remove_lines).map_err(|e| format!("/etc/pam.d/{s}: {e}"))?;
        }
    }
    let p = keyfile::key_path(Path::new(keyfile::KEY_DIR), user).map_err(|e| e.to_string())?;
    match std::fs::remove_file(&p) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
        _ => Ok(()),
    }
}

/// What is set up for `user`, readable without root.
pub fn status(user: &str) -> serde_json::Value {
    let key = keyfile::key_path(Path::new(keyfile::KEY_DIR), user)
        .ok()
        .and_then(|p| keyfile::read(&p, 0).ok());
    serde_json::json!({
        "helper": pam::helper_is_safe(Path::new(pam::HELPER)).is_ok(),
        "device": key.as_ref().map(|k| k.device_name.clone()),
        "deviceId": key.as_ref().map(|k| k.device_id.clone()),
        "sudo": pam::enabled("sudo"),
    })
}
