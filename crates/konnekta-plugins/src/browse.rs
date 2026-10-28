//! "Browse PC": a paired phone reads selected folders of this computer.
//!
//! Read-only; a phone needs the "Browse computer files" permission, and
//! `shareHome` (on by default) turns it off for every phone. Served folders: home,
//! download folder, Documents, Pictures, Music, Videos. Rules:
//! * a path must stay inside its folder after resolving symlinks and `..`;
//! * names starting with a dot are hidden and refused (`~/.ssh`, `~/.config`...);
//! * Konnekta's own data folder (private key) is never served;
//! * only regular files and folders are listed; nothing can be changed;
//! * a session ends after 1 hour, on stop, unpair or disconnect, and the
//!   local user sees a notification with a Stop button meanwhile.

use crate::store::JsonStore;
use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use konnekta_core::payload::PayloadSource;
use konnekta_core::proto::{Packet, types};
use konnekta_core::{CoreHandle, DeviceHandle, Permission, Plugin, PluginInfo};
use konnekta_platform::{DesktopNotification, Platform};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

pub const SESSION_TIME: Duration = Duration::from_secs(3600);
pub const SEARCH_TIME: Duration = Duration::from_secs(10);
pub const SEARCH_RESULTS: usize = 100;
const MAX_LIST: usize = 5000;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct BrowseConfig {
    pub share_home: bool,
}

impl Default for BrowseConfig {
    fn default() -> Self {
        Self { share_home: true }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub name: String,
    pub path: String,
    pub dir: bool,
    pub size: u64,
    pub modified: i64,
}

#[derive(Debug, Clone)]
pub struct Root {
    pub id: String,
    pub label: String,
    pub path: PathBuf,
}

/// The folders this computer serves.
pub fn default_roots(download_dir: &Path) -> Vec<Root> {
    let mut out = Vec::new();
    let Some(user) = directories::UserDirs::new() else {
        return out;
    };
    let home = user.home_dir().to_path_buf();
    let mut add = |id: &str, label: &str, p: Option<&Path>| {
        if let Some(p) = p.filter(|p| p.is_dir()) {
            out.push(Root {
                id: id.into(),
                label: label.into(),
                path: p.to_path_buf(),
            });
        }
    };
    add("home", "Home", Some(&home));
    // A download folder that contains home (e.g. "/") or lives in a dot
    // folder is not offered.
    let dl_ok = !home.starts_with(download_dir)
        && !download_dir
            .components()
            .any(|c| matches!(c, Component::Normal(n) if n.to_string_lossy().starts_with('.')));
    add("downloads", "Download", dl_ok.then_some(download_dir));
    add("documents", "Documenti", user.document_dir());
    add("pictures", "Immagini", user.picture_dir());
    add("music", "Musica", user.audio_dir());
    add("videos", "Video", user.video_dir());
    out
}

/// Resolve `rel` inside `root`, enforcing every rule. Returns the canonical
/// path.
pub fn resolve(root: &Path, rel: &str, forbidden: &[PathBuf]) -> Result<PathBuf> {
    let rel = Path::new(rel.trim_start_matches(['/', '\\']));
    for c in rel.components() {
        match c {
            Component::Normal(n) if !n.to_string_lossy().starts_with('.') => {}
            Component::CurDir => {}
            _ => bail!("path not allowed"),
        }
    }
    let root = root.canonicalize()?;
    let full = root.join(rel).canonicalize().context("not found")?;
    if !allowed_target(&root, &full, forbidden) {
        bail!("path not allowed");
    }
    let meta = std::fs::metadata(&full)?;
    if !meta.is_file() && !meta.is_dir() {
        bail!("path not allowed");
    }
    Ok(full)
}

/// A resolved (canonical) path may be served: inside the root, not in a
/// hidden folder (symlinks may point into one), not in a forbidden folder.
fn allowed_target(root: &Path, full: &Path, forbidden: &[PathBuf]) -> bool {
    let Ok(inner) = full.strip_prefix(root) else {
        return false;
    };
    !inner
        .components()
        .any(|c| matches!(c, Component::Normal(n) if n.to_string_lossy().starts_with('.')))
        && !forbidden.iter().any(|f| {
            f.canonicalize()
                .map(|f| full.starts_with(f))
                .unwrap_or(false)
        })
}

fn entry(root: &Path, p: &Path, meta: &std::fs::Metadata) -> Option<Entry> {
    let name = p.file_name()?.to_string_lossy().into_owned();
    Some(Entry {
        // Paths on the wire always use `/`, whatever the host OS.
        path: p
            .strip_prefix(root)
            .ok()?
            .to_string_lossy()
            .replace('\\', "/"),
        name,
        dir: meta.is_dir(),
        size: if meta.is_file() { meta.len() } else { 0 },
        modified: meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0),
    })
}

pub fn list(root: &Path, rel: &str, forbidden: &[PathBuf]) -> Result<Vec<Entry>> {
    let dir = resolve(root, rel, forbidden)?;
    let root = root.canonicalize()?;
    if !dir.is_dir() {
        bail!("not a folder");
    }
    let mut out = Vec::new();
    for e in std::fs::read_dir(&dir)?.flatten().take(MAX_LIST) {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        // Follow symlinks only if they stay inside the root.
        let Ok(target) = e.path().canonicalize() else {
            continue;
        };
        if !allowed_target(&root, &target, forbidden) {
            continue;
        }
        let Ok(meta) = std::fs::metadata(&target) else {
            continue;
        };
        if !meta.is_file() && !meta.is_dir() {
            continue;
        }
        if let Some(mut en) = entry(&root, &e.path(), &meta) {
            en.name = name;
            out.push(en);
        }
    }
    out.sort_by(|a, b| {
        b.dir
            .cmp(&a.dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(out)
}

/// Find files whose name contains every word (any case). Skips hidden and
/// `node_modules` folders, never follows folder symlinks, stops after 10 s.
pub fn search(root: &Path, rel: &str, query: &str, forbidden: &[PathBuf]) -> Result<Vec<Entry>> {
    let start = resolve(root, rel, forbidden)?;
    let root = root.canonicalize()?;
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    if words.is_empty() {
        return Ok(vec![]);
    }
    let deadline = Instant::now() + SEARCH_TIME;
    let mut found: Vec<(usize, Entry)> = Vec::new();
    let mut stack = vec![start];
    while let Some(dir) = stack.pop() {
        if Instant::now() > deadline || found.len() >= SEARCH_RESULTS * 4 {
            break;
        }
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || name == "node_modules" {
                continue;
            }
            let Ok(ft) = e.file_type() else { continue };
            let path = e.path();
            if forbidden.iter().any(|f| path.starts_with(f)) {
                continue;
            }
            if ft.is_dir() {
                stack.push(path.clone());
            }
            let lower = name.to_lowercase();
            if words.iter().all(|w| lower.contains(w))
                && (ft.is_file() || ft.is_dir())
                && let (Ok(meta), Some(score)) = (
                    e.metadata(),
                    Some(usize::from(!lower.starts_with(&words[0]))),
                )
                && let Some(en) = entry(&root, &path, &meta)
            {
                found.push((score, en));
            }
        }
    }
    found.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then_with(|| a.1.name.len().cmp(&b.1.name.len()))
    });
    Ok(found
        .into_iter()
        .take(SEARCH_RESULTS)
        .map(|(_, e)| e)
        .collect())
}

pub struct BrowsePlugin {
    platform: Platform,
    core: OnceLock<CoreHandle>,
    pub config: JsonStore<BrowseConfig>,
    sessions: Mutex<HashMap<String, Instant>>,
    /// Extra folders never served (Konnekta data dir).
    forbidden: Mutex<Vec<PathBuf>>,
    roots_override: Mutex<Option<Vec<Root>>>,
}

impl BrowsePlugin {
    pub fn new(platform: Platform) -> Self {
        Self {
            platform,
            core: OnceLock::new(),
            config: JsonStore::default(),
            sessions: Mutex::default(),
            forbidden: Mutex::default(),
            roots_override: Mutex::new(None),
        }
    }

    /// Tests: serve these roots instead of the user's folders.
    pub fn with_roots(self, roots: Vec<Root>) -> Self {
        *self.roots_override.lock().unwrap() = Some(roots);
        self
    }

    fn roots(&self) -> Vec<Root> {
        if let Some(r) = self.roots_override.lock().unwrap().clone() {
            return r;
        }
        let dl = self
            .core
            .get()
            .map(CoreHandle::download_dir)
            .unwrap_or_default();
        default_roots(&dl)
    }

    fn root(&self, id: &str) -> Result<Root> {
        self.roots()
            .into_iter()
            .find(|r| r.id == id)
            .context("unknown folder")
    }

    fn session_ok(&self, dev: &DeviceHandle) -> bool {
        let mut s = self.sessions.lock().unwrap();
        match s.get(dev.id()) {
            Some(t) if t.elapsed() < SESSION_TIME => true,
            Some(_) => {
                s.remove(dev.id());
                false
            }
            None => false,
        }
    }

    fn begin_session(&self, dev: &DeviceHandle) {
        let new = self
            .sessions
            .lock()
            .unwrap()
            .insert(dev.id().to_owned(), Instant::now())
            .is_none();
        if new {
            dev.emit("browse", "started", json!({}));
            let core = self.core.get().cloned();
            let id = dev.id().to_owned();
            let _ = self.platform.notifier.show(
                DesktopNotification {
                    id: format!("browse-{id}"),
                    app_name: dev.name().into(),
                    title: "Accesso ai file".into(),
                    body: format!(
                        "{} sta sfogliando le cartelle di questo computer",
                        dev.name()
                    ),
                    actions: vec![("stop".into(), "Stop".into())],
                    urgent: false,
                },
                Some(Box::new(move |a| {
                    if a != "stop" {
                        return;
                    }
                    let (Some(core), Ok(rt)) = (core, tokio::runtime::Handle::try_current()) else {
                        return;
                    };
                    rt.spawn(async move {
                        let _ = core
                            .plugin_action(None, "browse", "stop", json!({ "deviceId": id }))
                            .await;
                    });
                })),
            );
        }
    }

    fn end_session(&self, device_id: &str) -> bool {
        let ended = self.sessions.lock().unwrap().remove(device_id).is_some();
        if ended {
            self.platform.notifier.close(&format!("browse-{device_id}"));
            if let Some(c) = self.core.get() {
                c.emit_plugin(Some(device_id), "browse", "stopped", json!({}));
            }
        }
        ended
    }

    async fn reply(dev: &DeviceHandle, body: Value) -> Result<()> {
        dev.send(Packet::new(types::BROWSE, body)).await
    }

    async fn serve(&self, dev: &DeviceHandle, p: &Packet) -> Result<()> {
        let action = p.get_str("action").unwrap_or_default();
        if action == "stop" {
            self.end_session(dev.id());
            return Ok(());
        }
        if !self.config.with(|c| c.share_home) {
            return Self::reply(dev, json!({ "action": "error", "error": "La condivisione delle cartelle è disattivata sul computer." })).await;
        }
        if !self.session_ok(dev) {
            self.begin_session(dev);
        }
        let forbidden = self.forbidden.lock().unwrap().clone();
        let root_id = p.get_str("root").unwrap_or("home").to_owned();
        let rel = p.get_str("path").unwrap_or("").to_owned();
        let result: Result<Value> = async {
            match action {
                "roots" => Ok(json!({
                    "action": "roots",
                    "roots": self.roots().iter().map(|r| json!({ "id": r.id, "label": r.label })).collect::<Vec<_>>(),
                })),
                "list" => {
                    let root = self.root(&root_id)?;
                    let entries = tokio::task::spawn_blocking(move || list(&root.path, &rel, &forbidden)).await??;
                    Ok(json!({ "action": "list", "root": root_id, "path": p.get_str("path").unwrap_or(""), "entries": entries }))
                }
                "search" => {
                    let root = self.root(&root_id)?;
                    let q = p.get_str("query").unwrap_or("").to_owned();
                    let entries = tokio::task::spawn_blocking(move || search(&root.path, &rel, &q, &forbidden)).await??;
                    Ok(json!({ "action": "search", "root": root_id, "entries": entries }))
                }
                "get" => {
                    let root = self.root(&root_id)?;
                    let path = resolve(&root.path, &rel, &forbidden)?;
                    if !path.is_file() {
                        bail!("not a file");
                    }
                    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    let packet = Packet::new(types::BROWSE, json!({ "action": "file", "root": root_id, "path": rel, "name": name }));
                    dev.send_with_payload(packet, PayloadSource::File(path), None).await?;
                    Ok(Value::Null)
                }
                other => bail!("unknown browse action {other}"),
            }
        }
        .await;
        match result {
            Ok(Value::Null) => Ok(()),
            Ok(v) => Self::reply(dev, v).await,
            Err(e) => Self::reply(dev, json!({ "action": "error", "error": e.to_string() })).await,
        }
    }
}

#[async_trait]
impl Plugin for BrowsePlugin {
    fn info(&self) -> PluginInfo {
        PluginInfo {
            id: "browse",
            name: "Browse PC",
            description: "Let the phone read selected folders of this computer (read-only)",
            permission: Permission::BrowseFiles,
            incoming: &[types::BROWSE_REQUEST],
            outgoing: &[types::BROWSE],
        }
    }

    async fn start(self: Arc<Self>, core: CoreHandle) {
        self.config.attach(&core, "browse", "config.json");
        if let Some(d) = core
            .plugin_data_dir("browse")
            .and_then(|d| d.parent()?.parent().map(Path::to_path_buf))
        {
            self.forbidden.lock().unwrap().push(d);
        }
        let _ = self.core.set(core);
    }

    async fn on_disconnected(&self, id: &str) {
        self.end_session(id);
    }

    async fn handle_packet(&self, dev: &DeviceHandle, p: Packet) -> Result<()> {
        self.serve(dev, &p).await
    }

    async fn handle_action(
        &self,
        _: Option<&DeviceHandle>,
        action: &str,
        args: Value,
    ) -> Result<Value> {
        match action {
            "sessions" => {
                let s = self.sessions.lock().unwrap();
                Ok(json!(
                    s.iter()
                        .map(|(d, t)| json!({ "deviceId": d, "seconds": t.elapsed().as_secs() }))
                        .collect::<Vec<_>>()
                ))
            }
            "stop" => {
                match args.get("deviceId").and_then(Value::as_str) {
                    Some(id) => {
                        self.end_session(id);
                    }
                    None => {
                        let ids: Vec<String> =
                            self.sessions.lock().unwrap().keys().cloned().collect();
                        for id in ids {
                            self.end_session(&id);
                        }
                    }
                }
                Ok(Value::Null)
            }
            "getConfig" => Ok(serde_json::to_value(self.config.get())?),
            "setConfig" => {
                let c: BrowseConfig = crate::args(args)?;
                if !c.share_home {
                    let ids: Vec<String> = self.sessions.lock().unwrap().keys().cloned().collect();
                    for id in ids {
                        self.end_session(&id);
                    }
                }
                self.config.update(|v| *v = c);
                Ok(Value::Null)
            }
            _ => bail!("unknown action {action}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree() -> (tempfile::TempDir, PathBuf) {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("home");
        std::fs::create_dir_all(root.join("Documents/Fatture")).unwrap();
        std::fs::create_dir_all(root.join(".ssh")).unwrap();
        std::fs::create_dir_all(root.join("node_modules/pkg")).unwrap();
        std::fs::write(root.join(".ssh/id_ed25519"), b"secret").unwrap();
        std::fs::write(root.join("Documents/Fatture/Invoice-2026-09.pdf"), b"pdf").unwrap();
        std::fs::write(root.join("node_modules/pkg/invoice 2026.js"), b"x").unwrap();
        std::fs::write(t.path().join("outside.txt"), b"x").unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(t.path().join("outside.txt"), root.join("escape.txt"))
                .unwrap();
            std::os::unix::fs::symlink(root.join(".ssh"), root.join("keys")).unwrap();
        }
        (t, root)
    }

    #[test]
    fn hidden_and_escaping_paths_are_refused() {
        let (_t, root) = tree();
        assert!(resolve(&root, ".ssh/id_ed25519", &[]).is_err());
        assert!(resolve(&root, "../outside.txt", &[]).is_err());
        assert!(resolve(&root, "Documents/../../outside.txt", &[]).is_err());
        #[cfg(unix)]
        {
            assert!(
                resolve(&root, "escape.txt", &[]).is_err(),
                "symlink out of the folder"
            );
            assert!(
                resolve(&root, "keys/id_ed25519", &[]).is_err(),
                "symlink into a hidden folder"
            );
        }
        assert!(resolve(&root, "Documents/Fatture/Invoice-2026-09.pdf", &[]).is_ok());
        assert!(
            resolve(&root, "Documents", &[root.join("Documents")]).is_err(),
            "forbidden folder"
        );
        let names: Vec<String> = list(&root, "", &[])
            .unwrap()
            .into_iter()
            .map(|e| e.name)
            .collect();
        assert!(
            !names
                .iter()
                .any(|n| n.starts_with('.') || n == "escape.txt" || n == "keys")
        );
        assert!(names.contains(&"Documents".to_string()));
    }

    #[test]
    fn search_finds_words_and_skips_node_modules() {
        let (_t, root) = tree();
        let r = search(&root, "", "invoice 2026", &[]).unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].path, "Documents/Fatture/Invoice-2026-09.pdf");
        assert!(search(&root, "", "id_ed25519", &[]).unwrap().is_empty());
    }
}
