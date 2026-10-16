//! Persistent storage of the local identity.

use crate::{CryptoError, LocalIdentity, Result, generate_device_id};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Where the private key lives.
pub trait KeyStore: Send + Sync {
    fn load_or_create(&self) -> Result<LocalIdentity>;
    /// Discard the identity and generate a new one (key rotation). All paired
    /// devices will need to pair again.
    fn rotate(&self) -> Result<LocalIdentity>;
}

/// Atomically write a file readable only by the current user.
pub fn write_private_file(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let tmp = path.with_extension("tmp");
    {
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut f = opts.open(&tmp)?;
        f.write_all(data)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Stores `identity.json` (device id + certificate, public) and the PKCS#8
/// private key either in the OS keyring (`os-keyring` feature) or in
/// `private_key.der` with mode 0600.
pub struct FileKeyStore {
    dir: PathBuf,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct IdentityFile {
    device_id: String,
    certificate: String,
}

impl FileKeyStore {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    fn identity_path(&self) -> PathBuf {
        self.dir.join("identity.json")
    }

    fn key_path(&self) -> PathBuf {
        self.dir.join("private_key.der")
    }

    fn save(&self, id: &LocalIdentity) -> Result<()> {
        save_key(&self.key_path(), &id.device_id, id.key_der())?;
        let f = IdentityFile {
            device_id: id.device_id.clone(),
            certificate: B64.encode(&id.cert_der),
        };
        let json = serde_json_like(&f);
        write_private_file(&self.identity_path(), json.as_bytes())?;
        Ok(())
    }

    fn load(&self) -> Result<Option<LocalIdentity>> {
        let Ok(raw) = std::fs::read_to_string(self.identity_path()) else {
            return Ok(None);
        };
        let f: IdentityFile = parse_identity(&raw)?;
        let cert = B64
            .decode(f.certificate)
            .map_err(|e| CryptoError::KeyStore(e.to_string()))?;
        let Some(key) = load_key(&self.key_path(), &f.device_id)? else {
            return Ok(None);
        };
        Ok(Some(LocalIdentity::from_parts(f.device_id, cert, key)?))
    }
}

impl KeyStore for FileKeyStore {
    fn load_or_create(&self) -> Result<LocalIdentity> {
        if let Some(id) = self.load()? {
            return Ok(id);
        }
        self.rotate()
    }

    fn rotate(&self) -> Result<LocalIdentity> {
        let id = LocalIdentity::generate(&generate_device_id())?;
        self.save(&id)?;
        tracing::info!(device_id = %id.device_id, "generated new device identity");
        Ok(id)
    }
}

// Tiny hand-rolled (de)serialisation to avoid pulling serde_json in this crate.
fn serde_json_like(f: &IdentityFile) -> String {
    format!(
        "{{\"device_id\":\"{}\",\"certificate\":\"{}\"}}",
        f.device_id, f.certificate
    )
}

fn parse_identity(raw: &str) -> Result<IdentityFile> {
    fn field(raw: &str, name: &str) -> Option<String> {
        let key = format!("\"{name}\":\"");
        let start = raw.find(&key)? + key.len();
        let end = raw[start..].find('"')? + start;
        Some(raw[start..end].to_owned())
    }
    Ok(IdentityFile {
        device_id: field(raw, "device_id")
            .ok_or_else(|| CryptoError::KeyStore("corrupt identity.json".into()))?,
        certificate: field(raw, "certificate")
            .ok_or_else(|| CryptoError::KeyStore("corrupt identity.json".into()))?,
    })
}

#[cfg(feature = "os-keyring")]
const KEYRING_SERVICE: &str = "konnekta";

#[cfg(feature = "os-keyring")]
fn save_key(_path: &Path, device_id: &str, key: &[u8]) -> Result<()> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, device_id)
        .map_err(|e| CryptoError::KeyStore(e.to_string()))?;
    entry
        .set_secret(key)
        .map_err(|e| CryptoError::KeyStore(e.to_string()))
}

#[cfg(feature = "os-keyring")]
fn load_key(_path: &Path, device_id: &str) -> Result<Option<Vec<u8>>> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, device_id)
        .map_err(|e| CryptoError::KeyStore(e.to_string()))?;
    match entry.get_secret() {
        Ok(k) => Ok(Some(k)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(CryptoError::KeyStore(e.to_string())),
    }
}

#[cfg(not(feature = "os-keyring"))]
fn save_key(path: &Path, _device_id: &str, key: &[u8]) -> Result<()> {
    write_private_file(path, key)?;
    Ok(())
}

#[cfg(not(feature = "os-keyring"))]
fn load_key(path: &Path, _device_id: &str) -> Result<Option<Vec<u8>>> {
    match std::fs::read(path) {
        Ok(k) => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mode = std::fs::metadata(path)?.permissions().mode();
                if mode & 0o077 != 0 {
                    tracing::warn!(
                        "private key file is accessible by other users; fixing permissions"
                    );
                    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
                }
            }
            Ok(Some(k))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persists_and_reloads() {
        let dir = tempfile::tempdir().unwrap();
        let store = FileKeyStore::new(dir.path());
        let a = store.load_or_create().unwrap();
        let b = store.load_or_create().unwrap();
        assert_eq!(a.device_id, b.device_id);
        assert_eq!(a.cert_der, b.cert_der);
        let c = store.rotate().unwrap();
        assert_ne!(a.device_id, c.device_id);
        #[cfg(all(unix, not(feature = "os-keyring")))]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.path().join("private_key.der"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }
}
