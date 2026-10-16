//! Small JSON persistence helper for plugin settings.

use konnekta_core::CoreHandle;
use serde::{Serialize, de::DeserializeOwned};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

pub struct JsonStore<T> {
    path: OnceLock<PathBuf>,
    value: Mutex<T>,
}

impl<T: Serialize + DeserializeOwned + Default + Clone> Default for JsonStore<T> {
    fn default() -> Self {
        Self::new(T::default())
    }
}

impl<T: Serialize + DeserializeOwned + Default + Clone> JsonStore<T> {
    pub fn new(initial: T) -> Self {
        Self { path: OnceLock::new(), value: Mutex::new(initial) }
    }

    /// Bind to `<data>/plugins/<plugin>/<file>` and load existing content.
    pub fn attach(&self, core: &CoreHandle, plugin: &str, file: &str) {
        let Some(dir) = core.plugin_data_dir(plugin) else { return };
        let path = dir.join(file);
        if let Ok(raw) = std::fs::read(&path) {
            match serde_json::from_slice::<T>(&raw) {
                Ok(v) => *self.value.lock().unwrap() = v,
                Err(e) => tracing::warn!("ignoring corrupt {}: {e}", path.display()),
            }
        }
        let _ = self.path.set(path);
    }

    pub fn get(&self) -> T {
        self.value.lock().unwrap().clone()
    }

    pub fn with<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        f(&self.value.lock().unwrap())
    }

    pub fn update<R>(&self, f: impl FnOnce(&mut T) -> R) -> R {
        let (r, snapshot) = {
            let mut v = self.value.lock().unwrap();
            let r = f(&mut v);
            (r, v.clone())
        };
        if let Some(path) = self.path.get() {
            match serde_json::to_vec_pretty(&snapshot) {
                Ok(data) => {
                    if let Err(e) = konnekta_crypto_write(path, &data) {
                        tracing::error!("cannot save {}: {e}", path.display());
                    }
                }
                Err(e) => tracing::error!("cannot serialize settings: {e}"),
            }
        }
        r
    }
}

fn konnekta_crypto_write(path: &std::path::Path, data: &[u8]) -> std::io::Result<()> {
    // Settings may contain commands: keep them private to the user.
    let tmp = path.with_extension("tmp");
    {
        let mut o = std::fs::OpenOptions::new();
        o.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            o.mode(0o600);
        }
        use std::io::Write;
        o.open(&tmp)?.write_all(data)?;
    }
    std::fs::rename(tmp, path)
}
