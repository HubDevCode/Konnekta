//! Konnekta desktop app: runs the core in-process, exposes it to the web UI
//! through a single `api` command and forwards core events as `konnekta`
//! window events. The local control socket is also served so the `konnekta`
//! CLI keeps working while the app is open.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use konnekta_core::{Config, Konnekta};
use serde_json::Value;
use tauri::menu::{Menu, MenuItem};
use tauri::image::Image;
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, State, WindowEvent};
use tauri_plugin_dialog::DialogExt;

struct AppState {
    k: Konnekta,
}

#[tauri::command]
async fn api(state: State<'_, AppState>, method: String, params: Value) -> Result<Value, String> {
    konnekta_core::api::call(&state.k, &method, params)
        .await
        .map_err(|e| format!("{e:#}"))
}

/// Setup assistant: what the media features still need on this computer.
#[tauri::command]
async fn setup_check() -> Value {
    tokio::task::spawn_blocking(konnekta_media::setup::check)
        .await
        .ok()
        .and_then(|r| serde_json::to_value(r).ok())
        .unwrap_or_default()
}

#[tauri::command]
async fn pick_files(app: AppHandle) -> Vec<String> {
    app.dialog()
        .file()
        .blocking_pick_files()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|p| p.into_path().ok())
        .map(|p| p.to_string_lossy().into_owned())
        .collect()
}

#[tauri::command]
async fn pick_folder(app: AppHandle) -> Option<String> {
    app.dialog()
        .file()
        .blocking_pick_folder()
        .and_then(|p| p.into_path().ok())
        .map(|p| p.to_string_lossy().into_owned())
}

fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

/// Desktop-shell preferences, stored next to the core config. All on by default:
/// tray icon, start with the session (tray only) and entry in the applications list.
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy)]
#[serde(rename_all = "camelCase", default)]
struct DesktopPrefs {
    tray: bool,
    autostart: bool,
    app_list: bool,
}

impl Default for DesktopPrefs {
    fn default() -> Self {
        Self { tray: true, autostart: true, app_list: true }
    }
}

static TRAY_ON: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

fn prefs_path() -> std::path::PathBuf {
    Config::default_data_dir().join("desktop.json")
}

fn load_prefs() -> DesktopPrefs {
    std::fs::read(prefs_path())
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

/// Writes or removes the launcher and autostart entries. Run at every start so
/// the entries follow the running executable and the current icon. Packages
/// (deb/rpm/NSIS) ship their own launcher; this covers portable binaries.
fn apply_desktop_entries(p: &DesktopPrefs) {
    #[cfg(target_os = "linux")]
    {
        let Some(home) = std::env::var_os("HOME").map(std::path::PathBuf::from) else {
            return;
        };
        let Ok(exe) = std::env::current_exe() else {
            return;
        };
        let data = std::env::var_os("XDG_DATA_HOME")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| home.join(".local/share"));
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| home.join(".config"));
        let entry = |icon: &str, exec_args: &str, extra: &str| {
            format!(
                "[Desktop Entry]\nType=Application\nName=Konnekta\nComment=Connect your phone and computer\nExec=\"{}\"{}\nIcon={}\nTerminal=false\nCategories=Utility;\nStartupWMClass=konnekta-desktop\n{}",
                exe.display(),
                exec_args,
                icon,
                extra
            )
        };
        let launcher = data.join("applications/konnekta.desktop");
        if p.app_list {
            // Every size the launchers ask for, so none falls back to an old icon.
            let icons: [(&str, &[u8]); 4] = [
                ("32x32", include_bytes!("../icons/32x32.png")),
                ("128x128", include_bytes!("../icons/128x128.png")),
                ("256x256", include_bytes!("../icons/128x128@2x.png")),
                ("512x512", include_bytes!("../icons/icon.png")),
            ];
            let mut icon_path = String::from("konnekta");
            for (size, bytes) in icons {
                let dir = data.join(format!("icons/hicolor/{size}/apps"));
                let _ = std::fs::create_dir_all(&dir);
                let file = dir.join("konnekta.png");
                let _ = std::fs::write(&file, bytes);
                if size == "256x256" {
                    icon_path = file.display().to_string();
                }
            }
            let _ = std::fs::create_dir_all(data.join("applications"));
            let _ = std::fs::write(&launcher, entry(&icon_path, "", ""));
            // Make menus pick up the refreshed icon and entry.
            let _ = std::process::Command::new("update-desktop-database")
                .arg(data.join("applications"))
                .output();
            let _ = std::process::Command::new("gtk-update-icon-cache")
                .args(["-q", "-t", "-f"])
                .arg(data.join("icons/hicolor"))
                .output();
        } else {
            let _ = std::fs::remove_file(&launcher);
        }
        let auto = config.join("autostart/konnekta.desktop");
        if p.autostart {
            let _ = std::fs::create_dir_all(config.join("autostart"));
            let _ = std::fs::write(&auto, entry("konnekta", " --background", "X-GNOME-Autostart-enabled=true\n"));
        } else {
            let _ = std::fs::remove_file(&auto);
        }
    }
    #[cfg(not(target_os = "linux"))]
    let _ = p;
}

#[tauri::command]
fn desktop_prefs_get() -> DesktopPrefs {
    load_prefs()
}

#[tauri::command]
fn desktop_prefs_set(app: AppHandle, prefs: DesktopPrefs) -> Result<(), String> {
    let path = prefs_path();
    let json = serde_json::to_vec_pretty(&prefs).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| format!("{e}"))?;
    TRAY_ON.store(prefs.tray, std::sync::atomic::Ordering::Relaxed);
    if let Some(t) = app.tray_by_id("konnekta") {
        let _ = t.set_visible(prefs.tray);
    }
    apply_desktop_entries(&prefs);
    Ok(())
}

const RELEASES_API: &str = "https://api.github.com/repos/HubDevCode/Konnekta/releases/latest";

fn curl(args: &[&str]) -> Result<Vec<u8>, String> {
    let out = std::process::Command::new("curl")
        .args(["-fsSL", "--max-time", "300", "-H", "User-Agent: konnekta-desktop"])
        .args(args)
        .output()
        .map_err(|e| format!("curl non disponibile: {e}"))?;
    if out.status.success() {
        Ok(out.stdout)
    } else {
        Err(format!("download non riuscito ({})", out.status))
    }
}

fn version_parts(v: &str) -> Vec<u64> {
    v.trim_start_matches('v')
        .split(['.', '-', '+'])
        .map_while(|p| p.parse().ok())
        .collect()
}

/// The release asset built for this system: a plain executable on Linux, the
/// `-setup.exe` installer on Windows.
fn pick_asset(assets: &[Value]) -> Option<(String, String)> {
    let arch = std::env::consts::ARCH;
    assets.iter().find_map(|a| {
        let name = a["name"].as_str()?;
        let url = a["browser_download_url"].as_str()?;
        let n = name.to_lowercase();
        let ok = if cfg!(target_os = "windows") {
            n.ends_with("-setup.exe")
        } else if cfg!(target_os = "linux") {
            n.starts_with("konnekta-desktop") && n.contains("linux") && n.contains(arch)
                && !n.ends_with(".sha256")
        } else {
            false
        };
        ok.then(|| (name.to_string(), url.to_string()))
    })
}

fn latest_release() -> Result<Value, String> {
    let body = curl(&["-H", "Accept: application/vnd.github+json", RELEASES_API]);
    match body {
        Ok(b) => serde_json::from_slice(&b).map_err(|e| e.to_string()),
        // No release published yet.
        Err(e) if e.contains("22") => Ok(Value::Null),
        Err(e) => Err(e),
    }
}

#[tauri::command]
async fn check_update() -> Result<Value, String> {
    tokio::task::spawn_blocking(|| {
        let current = env!("CARGO_PKG_VERSION");
        let rel = latest_release()?;
        let tag = rel["tag_name"].as_str().unwrap_or("");
        let available = !tag.is_empty() && version_parts(tag) > version_parts(current);
        let asset = rel["assets"].as_array().and_then(|a| pick_asset(a));
        Ok(serde_json::json!({
            "current": current,
            "latest": tag.trim_start_matches('v'),
            "available": available,
            "notes": rel["body"],
            "page": rel["html_url"],
            "installable": asset.is_some(),
        }))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn install_update(app: AppHandle) -> Result<(), String> {
    let handle = app.clone();
    tokio::task::spawn_blocking(move || {
        let rel = latest_release()?;
        let assets = rel["assets"].as_array().cloned().unwrap_or_default();
        let (name, url) =
            pick_asset(&assets).ok_or("Nessun pacchetto per questo sistema nella release")?;
        let bytes = curl(&[&url])?;
        // Verify against `<asset>.sha256` when the release publishes one.
        let sum_name = format!("{name}.sha256");
        if let Some(sum_url) = assets
            .iter()
            .find(|a| a["name"].as_str() == Some(sum_name.as_str()))
            .and_then(|a| a["browser_download_url"].as_str())
        {
            let expected = String::from_utf8_lossy(&curl(&[sum_url])?).to_string();
            let expected = expected.split_whitespace().next().unwrap_or("").to_lowercase();
            let actual = sha256_hex(&bytes)?;
            if expected != actual {
                return Err("Il file scaricato non corrisponde al checksum: aggiornamento annullato".into());
            }
        }
        #[cfg(not(target_os = "windows"))]
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        #[cfg(target_os = "windows")]
        {
            let setup = std::env::temp_dir().join(&name);
            std::fs::write(&setup, &bytes).map_err(|e| e.to_string())?;
            std::process::Command::new(&setup).spawn().map_err(|e| e.to_string())?;
        }
        #[cfg(not(target_os = "windows"))]
        {
            use std::os::unix::fs::PermissionsExt;
            let tmp = exe.with_extension("update");
            std::fs::write(&tmp, &bytes).map_err(|e| format!("cartella non scrivibile: {e}"))?;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))
                .map_err(|e| e.to_string())?;
            std::fs::rename(&tmp, &exe).map_err(|e| e.to_string())?;
            std::process::Command::new("sh")
                .arg("-c")
                .arg("sleep 1; exec \"$0\"")
                .arg(&exe)
                .spawn()
                .map_err(|e| e.to_string())?;
        }
        handle.exit(0);
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

fn sha256_hex(data: &[u8]) -> Result<String, String> {
    use std::process::Command;
    let file = std::env::temp_dir().join("konnekta-update.bin");
    std::fs::write(&file, data).map_err(|e| e.to_string())?;
    let out = if cfg!(target_os = "windows") {
        Command::new("certutil").args(["-hashfile"]).arg(&file).arg("SHA256").output()
    } else {
        Command::new("sha256sum").arg(&file).output()
    };
    let _ = std::fs::remove_file(&file);
    let out = out.map_err(|e| format!("verifica checksum impossibile: {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    let hash = if cfg!(target_os = "windows") {
        text.lines().nth(1).unwrap_or("").split_whitespace().collect::<String>()
    } else {
        text.split_whitespace().next().unwrap_or("").to_string()
    };
    Ok(hash.to_lowercase())
}

async fn start_core() -> anyhow::Result<Konnekta> {
    let dir = Config::default_data_dir();
    std::fs::create_dir_all(&dir)?;
    let cfg_path = dir.join("config.json");
    let cfg = Config::load(&cfg_path)?;
    cfg.save(&cfg_path)?;
    let k = Konnekta::builder(cfg)
        .data_dir(&dir)
        .plugins(konnekta_plugins::all(konnekta_platform::Platform::native()))
        .build()?;
    k.start().await?;
    let ipc_k = k.clone();
    tokio::spawn(async move {
        if let Err(e) = konnekta_core::ipc::listen(konnekta_core::ipc::endpoint(&dir), ipc_k).await
        {
            tracing::warn!("control socket unavailable: {e:#}");
        }
    });
    Ok(k)
}

fn main() {
    // WebKitGTK's DMA-BUF renderer kills the window with a Wayland protocol
    // error on some drivers (seen on NVIDIA + Hyprland). The user can still
    // opt back in by setting the variable.
    #[cfg(target_os = "linux")]
    if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
        // SAFETY: still single-threaded, before any runtime or GTK starts.
        unsafe { std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1") };
    }
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            show_main(app)
        }))
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let prefs = load_prefs();
            TRAY_ON.store(prefs.tray, std::sync::atomic::Ordering::Relaxed);
            apply_desktop_entries(&prefs);
            let k = tauri::async_runtime::block_on(start_core())?;
            app.manage(AppState { k: k.clone() });

            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let mut rx = k.subscribe();
                loop {
                    match rx.recv().await {
                        Ok(ev) => {
                            // Bring the window up for pairing requests and security alerts.
                            if matches!(
                                ev,
                                konnekta_core::Event::PairRequested { .. }
                                    | konnekta_core::Event::SecurityWarning { .. }
                            ) {
                                show_main(&handle);
                            }
                            let _ = handle.emit("konnekta", &ev);
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(_) => break,
                    }
                }
            });

            let show = MenuItem::with_id(app, "show", "Apri Konnekta", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Esci", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &quit])?;
            let mut tray = TrayIconBuilder::with_id("konnekta")
                .menu(&menu)
                .tooltip("Konnekta");
            tray = tray.icon(Image::from_bytes(include_bytes!("../icons/tray.png"))?);
            if !prefs.tray || !std::env::args().any(|a| a == "--background") {
                show_main(app.handle());
            }
            tray.show_menu_on_left_click(false)
            .on_tray_icon_event(|t, e| {
                if let TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } = e
                {
                    show_main(t.app_handle());
                }
            })
            .on_menu_event(|app, e| match e.id.as_ref() {
                "show" => show_main(app),
                "quit" => app.exit(0),
                _ => {}
            })
            .build(app)?;
            if !prefs.tray {
                if let Some(t) = app.tray_by_id("konnekta") {
                    let _ = t.set_visible(false);
                }
            }
            Ok(())
        })
        .on_window_event(|w, e| {
            // Closing the window keeps Konnekta running in the tray.
            if let WindowEvent::CloseRequested { api, .. } = e {
                // Without the tray there would be no way back, so quit instead.
                if TRAY_ON.load(std::sync::atomic::Ordering::Relaxed) {
                    let _ = w.hide();
                    api.prevent_close();
                } else {
                    w.app_handle().exit(0);
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            api,
            setup_check,
            pick_files,
            pick_folder,
            desktop_prefs_get,
            desktop_prefs_set,
            check_update,
            install_update
        ])
        .run(tauri::generate_context!())
        .expect("error while running Konnekta");
}
