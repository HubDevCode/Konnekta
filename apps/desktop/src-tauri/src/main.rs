//! Konnekta desktop app: runs the core in-process, exposes it to the web UI
//! through a single `api` command and forwards core events as `konnekta`
//! window events. The local control socket is also served so the `konnekta`
//! CLI keeps working while the app is open.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use konnekta_core::{Config, Konnekta};
use serde_json::Value;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
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
            if let Some(icon) = app.default_window_icon() {
                tray = tray.icon(icon.clone());
            }
            tray.on_menu_event(|app, e| match e.id.as_ref() {
                "show" => show_main(app),
                "quit" => app.exit(0),
                _ => {}
            })
            .build(app)?;
            Ok(())
        })
        .on_window_event(|w, e| {
            // Closing the window keeps Konnekta running in the tray.
            if let WindowEvent::CloseRequested { api, .. } = e {
                let _ = w.hide();
                api.prevent_close();
            }
        })
        .invoke_handler(tauri::generate_handler![api, pick_files, pick_folder])
        .run(tauri::generate_context!())
        .expect("error while running Konnekta");
}
