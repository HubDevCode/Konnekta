//! "Approva sudo con l'impronta" in one click (Linux).
//!
//! 1. `approve_enroll`: the phone makes its key after the fingerprint and
//!    signs the enrollment; the app shows the code to compare with the one
//!    on the phone.
//! 2. `approve_confirm`: with the password typed in the app, `sudo -S`
//!    runs this same program as root (`--approve-install <user> <key>`),
//!    which installs the PAM helper, writes the key and turns on `sudo`.
//!    No polkit agent is needed (Hyprland and other bare sessions have
//!    none), and the key stays here until that works, so a wrong password
//!    can simply be typed again.
//!
//! The installed helper is a copy of `konnekta-approve` when it sits next to
//! this program, else of this program itself: started under the name
//! `konnekta-approve` it only runs the PAM helper ([`early_main`]).

use serde_json::Value;
#[cfg(not(target_os = "linux"))]
use serde_json::json;

/// Before anything else starts: the PAM helper and the root steps.
/// Returns the exit code when one of them ran.
pub fn early_main() -> Option<i32> {
    #[cfg(target_os = "linux")]
    {
        imp::early_main()
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

#[tauri::command]
pub fn approve_status() -> Value {
    #[cfg(target_os = "linux")]
    {
        imp::status()
    }
    #[cfg(not(target_os = "linux"))]
    {
        json!({ "available": false })
    }
}

#[tauri::command]
pub async fn approve_enroll(
    state: tauri::State<'_, crate::AppState>,
    device_id: String,
) -> Result<Value, String> {
    #[cfg(target_os = "linux")]
    {
        // The phone can only be asked with the permission on.
        state
            .k
            .set_permission(&device_id, konnekta_core::Permission::Approve, true)
            .map_err(|e| e.to_string())?;
        tokio::task::spawn_blocking(move || imp::enroll(&device_id))
            .await
            .map_err(|e| e.to_string())?
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (state, device_id);
        Err("disponibile solo su Linux".into())
    }
}

#[tauri::command]
pub async fn approve_confirm(password: String) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    {
        tokio::task::spawn_blocking(move || imp::confirm(&password))
            .await
            .map_err(|e| e.to_string())?
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = password;
        Err("disponibile solo su Linux".into())
    }
}

#[tauri::command]
pub async fn approve_disable(password: String) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    {
        tokio::task::spawn_blocking(move || imp::disable(&password))
            .await
            .map_err(|e| e.to_string())?
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = password;
        Err("disponibile solo su Linux".into())
    }
}

#[cfg(target_os = "linux")]
mod imp {
    use konnekta_approve::client::socket_path;
    use konnekta_approve::helper::{self, hostname, user_uid};
    use konnekta_approve::key::KeyFile;
    use konnekta_approve::{enroll, keyfile, setup};
    use serde_json::{Value, json};
    use std::io::Write;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};
    use std::sync::Mutex;

    const INSTALL: &str = "--approve-install";
    const REMOVE: &str = "--approve-remove";

    /// The key waiting for the user to compare the code.
    static PENDING: Mutex<Option<KeyFile>> = Mutex::new(None);

    pub fn early_main() -> Option<i32> {
        let args: Vec<String> = std::env::args().collect();
        let name = args
            .first()
            .and_then(|a| Path::new(a).file_name())
            .and_then(|n| n.to_str());
        if name == Some("konnekta-approve") {
            return Some(pam_helper());
        }
        let r = match args.as_slice() {
            [_, step, user, key] if step == INSTALL => root_step(user, Some(key)),
            [_, step, user] if step == REMOVE => root_step(user, None),
            _ => return None,
        };
        if let Err(e) = &r {
            eprintln!("{e}");
        }
        Some(i32::from(r.is_err()))
    }

    /// Same as the `konnekta-approve` binary: no flags, no environment
    /// except the PAM items.
    fn pam_helper() -> i32 {
        use helper::{Outcome, PamEnv, Settings, run};
        match run(&PamEnv::from_env(), &Settings::system(), &mut |l| {
            println!("{l}")
        }) {
            Outcome::Approved => 0,
            Outcome::Fallback(reason) => {
                eprintln!("konnekta-approve: {reason}");
                1
            }
        }
    }

    /// Run as root by `sudo` (or `pkexec`): install `key` (a public key
    /// file) or, without one, remove. The user must be the one who asked.
    fn root_step(user: &str, key: Option<&String>) -> Result<(), String> {
        let asking = std::env::var("SUDO_UID")
            .or_else(|_| std::env::var("PKEXEC_UID"))
            .ok()
            .and_then(|u| u.parse::<u32>().ok())
            .ok_or("avvialo con sudo")?;
        if !keyfile::valid_user(user) || user == "root" || user_uid(user) != Some(asking) {
            return Err(format!("utente non valido: {user}"));
        }
        match key {
            Some(key) => setup::install(user, key, &helper_source()?),
            None => setup::remove(user),
        }
    }

    /// `konnekta-approve` next to this program if there is one, else this
    /// program.
    fn helper_source() -> Result<PathBuf, String> {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let sibling = exe.with_file_name("konnekta-approve");
        Ok(if sibling.is_file() { sibling } else { exe })
    }

    fn me() -> Result<(String, u32), String> {
        // SAFETY: getuid never fails.
        let uid = unsafe { libc::getuid() };
        let user = std::env::var("USER").map_err(|_| "utente sconosciuto")?;
        if user_uid(&user) != Some(uid) {
            return Err(format!("utente non valido: {user}"));
        }
        Ok((user, uid))
    }

    pub fn status() -> Value {
        let Ok((user, _)) = me() else {
            return json!({ "available": false });
        };
        let mut s = setup::status(&user);
        s["available"] = json!(Path::new("/usr/bin/sudo").exists());
        s
    }

    pub fn enroll(device_id: &str) -> Result<Value, String> {
        let (user, uid) = me()?;
        let settings = enroll::Settings {
            key_dir: PathBuf::from(keyfile::KEY_DIR),
            socket: socket_path(uid),
            user,
            uid,
            host: hostname(),
        };
        let kf = enroll::request_key(&settings, device_id, &mut |_| {})?;
        let out = json!({ "code": kf.key.code(), "device": kf.device_name });
        *PENDING.lock().unwrap() = Some(kf);
        Ok(out)
    }

    /// Run this program as root through `sudo -S`, the password on its
    /// stdin. Once approval is on, sudo may ask the phone first.
    fn sudo(password: &str, args: &[&str]) -> Result<(), String> {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let mut child = Command::new("sudo")
            .args(["-S", "-p", ""])
            .arg(exe)
            .args(args)
            .env("LC_ALL", "C")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("sudo non disponibile: {e}"))?;
        if let Some(mut stdin) = child.stdin.take() {
            // One try: sudo stops at the end of the input.
            let _ = stdin.write_all(format!("{password}\n").as_bytes());
        }
        let out = child.wait_with_output().map_err(|e| e.to_string())?;
        if out.status.success() {
            return Ok(());
        }
        let err = String::from_utf8_lossy(&out.stderr);
        let err = err
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with("Sorry, try again") && !l.starts_with("[sudo]"))
            .collect::<Vec<_>>()
            .join(" ");
        Err(if err.contains("incorrect password") || err.contains("no password") {
            "Password errata".into()
        } else if err.contains("sudoers") {
            "Il tuo utente non può usare sudo".into()
        } else if err.is_empty() {
            "configurazione non riuscita".into()
        } else {
            err
        })
    }

    pub fn confirm(password: &str) -> Result<(), String> {
        let (user, _) = me()?;
        // Kept until it works: a wrong password can be typed again.
        let key = PENDING
            .lock()
            .unwrap()
            .as_ref()
            .map(KeyFile::render)
            .ok_or("registra prima il telefono")?;
        sudo(password, &[INSTALL, &user, &key])?;
        PENDING.lock().unwrap().take();
        Ok(())
    }

    pub fn disable(password: &str) -> Result<(), String> {
        let (user, _) = me()?;
        sudo(password, &[REMOVE, &user])
    }
}
