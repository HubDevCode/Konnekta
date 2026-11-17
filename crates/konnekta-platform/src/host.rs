//! Programs of the system, started with the system's own environment.
//!
//! The AppImage starts Konnekta with `LD_LIBRARY_PATH`, `PATH`,
//! `GST_PLUGIN_SYSTEM_PATH`, `GIO_MODULE_DIR`, `PYTHONHOME`… pointing into
//! its mount (`$APPDIR`), for its own bundled libraries. Programs of the
//! system inherit them and then load the wrong libraries: `playerctl` dies
//! with `undefined symbol: g_once_init_leave_pointer`, and so may anything
//! else linked to GLib, GStreamer, Python… Every program Konnekta starts
//! goes through [`command`] (or [`clean`]), which takes the AppImage's
//! entries out again. Outside an AppImage nothing changes.

use std::ffi::{OsStr, OsString};
use std::sync::OnceLock;

/// Variables to change for programs of the system: `Some` = new value,
/// `None` = remove.
fn fixes() -> &'static [(OsString, Option<OsString>)] {
    static FIXES: OnceLock<Vec<(OsString, Option<OsString>)>> = OnceLock::new();
    FIXES.get_or_init(|| match std::env::var_os("APPDIR") {
        Some(appdir) if std::env::var_os("APPIMAGE").is_some() && !appdir.is_empty() => {
            compute(&appdir, std::env::vars_os())
        }
        _ => Vec::new(),
    })
}

/// The changes that remove `appdir` from an environment.
fn compute(
    appdir: &OsStr,
    vars: impl Iterator<Item = (OsString, OsString)>,
) -> Vec<(OsString, Option<OsString>)> {
    let appdir = appdir.to_string_lossy();
    let appdir = appdir.trim_end_matches('/');
    let inside = |entry: &str| entry == appdir || entry.starts_with(&format!("{appdir}/"));
    let mut out = Vec::new();
    for (key, value) in vars {
        // Konnekta's own markers stay: nothing else reads them.
        if matches!(key.to_str(), Some("APPDIR" | "APPIMAGE" | "ARGV0" | "OWD")) {
            continue;
        }
        let Some(v) = value.to_str() else { continue };
        if !v.contains(appdir) {
            continue;
        }
        // Lists of paths keep the entries outside the AppImage; a single
        // path into it (PYTHONHOME, GTK_EXE_PREFIX…) goes away.
        let kept: Vec<&str> = v
            .split(':')
            .filter(|e| !e.is_empty() && !inside(e))
            .collect();
        let new = (!kept.is_empty()).then(|| OsString::from(kept.join(":")));
        out.push((key, new));
    }
    out
}

/// Whether Konnekta runs from an AppImage that changed the environment.
pub fn in_appimage() -> bool {
    !fixes().is_empty()
}

/// Give a command the system's environment.
pub fn clean(cmd: &mut std::process::Command) -> &mut std::process::Command {
    for (k, v) in fixes() {
        match v {
            Some(v) => cmd.env(k, v),
            None => cmd.env_remove(k),
        };
    }
    cmd
}

/// Give an async command the system's environment.
pub fn clean_async(cmd: &mut tokio::process::Command) -> &mut tokio::process::Command {
    for (k, v) in fixes() {
        match v {
            Some(v) => cmd.env(k, v),
            None => cmd.env_remove(k),
        };
    }
    cmd
}

/// A program of the system with the system's environment.
pub fn command(program: impl AsRef<OsStr>) -> std::process::Command {
    let mut c = std::process::Command::new(program);
    clean(&mut c);
    c
}

/// An async program of the system with the system's environment.
pub fn command_async(program: impl AsRef<OsStr>) -> tokio::process::Command {
    let mut c = tokio::process::Command::new(program);
    clean_async(&mut c);
    c
}

/// The `PATH` the system's programs see (the AppImage's folders left out).
pub fn path() -> Option<OsString> {
    match fixes().iter().find(|(k, _)| k == "PATH") {
        Some((_, v)) => v.clone(),
        None => std::env::var_os("PATH"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, &str)]) -> Vec<(OsString, OsString)> {
        pairs
            .iter()
            .map(|(k, v)| (OsString::from(k), OsString::from(v)))
            .collect()
    }

    #[test]
    fn appimage_entries_are_taken_out() {
        let d = "/tmp/.mount_KonnekX";
        let vars = env(&[
            (
                "LD_LIBRARY_PATH",
                "/tmp/.mount_KonnekX/usr/lib/:/tmp/.mount_KonnekX/lib/",
            ),
            (
                "PATH",
                "/tmp/.mount_KonnekX/usr/bin/:/usr/local/bin:/usr/bin",
            ),
            ("XDG_DATA_DIRS", "/tmp/.mount_KonnekX/usr/share/:/usr/share"),
            ("PYTHONHOME", "/tmp/.mount_KonnekX/usr/"),
            (
                "GST_PLUGIN_SYSTEM_PATH_1_0",
                "/tmp/.mount_KonnekX/usr/lib/gstreamer-1.0:",
            ),
            ("HOME", "/home/u"),
            ("APPIMAGE", "/home/u/Konnekta.AppImage"),
            ("APPDIR", "/tmp/.mount_KonnekX"),
            // Another mount whose name only starts the same way stays.
            ("OTHER", "/tmp/.mount_KonnekXYZ/bin"),
        ]);
        let mut f = compute(OsStr::new(d), vars.into_iter());
        f.sort();
        let get = |k: &str| f.iter().find(|(x, _)| x == k).map(|(_, v)| v.clone());
        assert_eq!(get("LD_LIBRARY_PATH"), Some(None));
        assert_eq!(get("PATH"), Some(Some("/usr/local/bin:/usr/bin".into())));
        assert_eq!(get("XDG_DATA_DIRS"), Some(Some("/usr/share".into())));
        assert_eq!(get("PYTHONHOME"), Some(None));
        assert_eq!(get("GST_PLUGIN_SYSTEM_PATH_1_0"), Some(None));
        assert_eq!(get("OTHER"), Some(Some("/tmp/.mount_KonnekXYZ/bin".into())));
        assert_eq!(get("HOME"), None);
        assert_eq!(get("APPIMAGE"), None);
        assert_eq!(get("APPDIR"), None);
    }

    #[test]
    fn nothing_changes_outside_an_appimage() {
        // The test runner is not an AppImage.
        if std::env::var_os("APPIMAGE").is_none() {
            assert!(!in_appimage());
            assert_eq!(path(), std::env::var_os("PATH"));
        }
    }
}
