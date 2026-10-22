//! The trust anchor: `/etc/konnekta/approve/<user>.pub`, written only by
//! root (`sudo konnekta approve enroll`). The path is fixed in the helper:
//! no flag or environment variable can point it to a key the user controls.

use crate::key::{KeyError, KeyFile, MAX_KEY_FILE};
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

pub const KEY_DIR: &str = "/etc/konnekta/approve";

/// Local user names: 1..=32 of `[A-Za-z0-9._-]`, not starting with `-` or `.`.
pub fn valid_user(u: &str) -> bool {
    !u.is_empty()
        && u.len() <= 32
        && !u.starts_with(['-', '.'])
        && u.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

pub fn key_path(dir: &Path, user: &str) -> Result<PathBuf, KeyError> {
    if !valid_user(user) {
        return Err(KeyError("invalid user name"));
    }
    Ok(dir.join(format!("{user}.pub")))
}

/// Every folder from `dir` up to `/` must be a real folder owned by `owner`
/// or root, and not writable by group or others unless it is a root-owned
/// sticky folder (like `/tmp`, for tests).
fn check_ancestors(dir: &Path, owner: u32) -> Result<(), KeyError> {
    let mut p = Some(dir);
    while let Some(d) = p {
        let m = std::fs::symlink_metadata(d).map_err(|_| KeyError("key folder missing"))?;
        if !m.file_type().is_dir() {
            return Err(KeyError("key folder is not a real folder"));
        }
        if m.uid() != owner && m.uid() != 0 {
            return Err(KeyError("key folder has the wrong owner"));
        }
        let writable = m.mode() & 0o022 != 0;
        let sticky_root = m.mode() & 0o1000 != 0 && m.uid() == 0;
        if writable && !sticky_root {
            return Err(KeyError("key folder is writable by others"));
        }
        p = d.parent();
    }
    Ok(())
}

/// Read and check the key file. `owner` is 0 in production.
pub fn read(path: &Path, owner: u32) -> Result<KeyFile, KeyError> {
    let dir = path.parent().ok_or(KeyError("bad key path"))?;
    check_ancestors(dir, owner)?;
    let before = std::fs::symlink_metadata(path).map_err(|_| KeyError("no key file"))?;
    if !before.file_type().is_file() {
        return Err(KeyError("key file is not a regular file"));
    }
    if before.uid() != owner {
        return Err(KeyError("key file has the wrong owner"));
    }
    if before.mode() & 0o022 != 0 {
        return Err(KeyError("key file is writable by others"));
    }
    if before.len() > MAX_KEY_FILE {
        return Err(KeyError("key file too large"));
    }
    let mut f = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map_err(|_| KeyError("cannot open key file"))?;
    let now = f.metadata().map_err(|_| KeyError("cannot stat key file"))?;
    if (now.dev(), now.ino()) != (before.dev(), before.ino()) {
        return Err(KeyError("key file changed while checking it"));
    }
    let mut text = String::new();
    (&mut f)
        .take(MAX_KEY_FILE + 1)
        .read_to_string(&mut text)
        .map_err(|_| KeyError("key file is not text"))?;
    KeyFile::parse(&text)
}

/// Write the key file atomically (temp file, 0644, fsync, rename) and make
/// the folder 0755. Run as root by `konnekta approve enroll`.
pub fn write(dir: &Path, user: &str, key: &KeyFile) -> std::io::Result<PathBuf> {
    let path = key_path(dir, user).map_err(|e| std::io::Error::other(e.0))?;
    std::fs::create_dir_all(dir)?;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o755))?;
    let tmp = dir.join(format!(".{user}.pub.tmp"));
    let _ = std::fs::remove_file(&tmp);
    {
        let mut f = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o644)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&tmp)?;
        f.write_all(key.render().as_bytes())?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, &path)?;
    if let Ok(d) = File::open(dir) {
        let _ = d.sync_all();
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::key::PublicKey;
    use crate::key::test_key::Phone;

    fn uid() -> u32 {
        unsafe { libc::getuid() }
    }

    fn keyfile() -> KeyFile {
        KeyFile {
            device_id: "0123456789abcdef0123456789abcdef".into(),
            device_name: "Pixel".into(),
            enrolled: 1,
            key: PublicKey::from_spki_der(&Phone::new().spki()).unwrap(),
        }
    }

    fn setup() -> (tempfile::TempDir, PathBuf) {
        let t = tempfile::Builder::new()
            .prefix("kn-approve")
            .tempdir()
            .unwrap();
        std::fs::set_permissions(t.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
        // Real path: the folder check refuses symlinked ancestors (/var on macOS).
        let dir = t.path().canonicalize().unwrap().join("approve");
        (t, dir)
    }

    #[test]
    fn written_file_reads_back() {
        let (_t, dir) = setup();
        let k = keyfile();
        let p = write(&dir, "naitsirk", &k).unwrap();
        assert_eq!(std::fs::metadata(&p).unwrap().mode() & 0o777, 0o644);
        assert_eq!(read(&p, uid()).unwrap(), k);
        assert!(read(&p, uid() + 1).is_err(), "wrong owner");
    }

    #[test]
    fn unsafe_files_are_refused() {
        let (_t, dir) = setup();
        let p = write(&dir, "u", &keyfile()).unwrap();
        // Writable by others.
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o666)).unwrap();
        assert_eq!(
            read(&p, uid()),
            Err(KeyError("key file is writable by others"))
        );
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o644)).unwrap();
        // Folder writable by others.
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o777)).unwrap();
        assert_eq!(
            read(&p, uid()),
            Err(KeyError("key folder is writable by others"))
        );
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        // A symlink to a key elsewhere.
        let link = dir.join("v.pub");
        std::os::unix::fs::symlink(&p, &link).unwrap();
        assert_eq!(
            read(&link, uid()),
            Err(KeyError("key file is not a regular file"))
        );
        assert!(read(&dir.join("nobody.pub"), uid()).is_err());
    }

    #[test]
    fn user_names() {
        assert!(valid_user("naitsirk") && valid_user("a.b_c-d"));
        for bad in ["", "-r", ".hidden", "../etc", "a/b", "x y", &"u".repeat(33)] {
            assert!(!valid_user(bad), "{bad}");
        }
    }
}
