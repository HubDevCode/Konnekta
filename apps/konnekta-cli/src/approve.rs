//! `konnekta approve`: set up fingerprint approval of `sudo` / polkit.
//!
//! * `sudo konnekta approve enroll <phone>` — the phone makes a key that
//!   needs the fingerprint, signs the enrollment, and the user types the
//!   code the phone shows; then root writes `/etc/konnekta/approve/<user>.pub`.
//! * `sudo konnekta approve enable sudo` — adds the PAM line.
//! * `konnekta approve status`, `sudo konnekta approve remove`, `disable`.
//!
//! Enroll only in a session you trust: code running as your user could type
//! the code for a key of its own (see docs/SICUREZZA.md).

use anyhow::{Context, Result, bail};
use clap::Subcommand;
use konnekta_approve::client::socket_path;
use konnekta_approve::helper::{hostname, now_secs, user_uid};
use konnekta_approve::{enroll, keyfile, pam};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

pub const DEFAULT_HELPER: &str = "/usr/lib/konnekta/konnekta-approve";

#[derive(Subcommand)]
pub enum ApproveCmd {
    /// Register the phone that approves sudo / polkit for you (run with sudo).
    Enroll { device: String },
    /// Forget the phone key (run with sudo).
    Remove,
    /// Show the enrolled phone and the PAM services that use it.
    Status,
    /// Let the phone approve a PAM service: sudo, sudo-i, polkit-1 (run with sudo).
    Enable {
        service: String,
        /// Installed helper; must be root-owned in a root-owned folder.
        #[arg(long, default_value = DEFAULT_HELPER)]
        helper: PathBuf,
    },
    /// Stop asking the phone for a PAM service (run with sudo).
    Disable { service: String },
}

fn need_root() -> Result<()> {
    // SAFETY: geteuid never fails.
    if unsafe { libc::geteuid() } != 0 {
        bail!("eseguilo con sudo");
    }
    Ok(())
}

/// The user who ran sudo.
fn sudo_user() -> Result<(String, u32)> {
    let user = std::env::var("SUDO_USER").context("eseguilo con sudo dal tuo account")?;
    if !keyfile::valid_user(&user) || user == "root" {
        bail!("utente non valido: {user}");
    }
    let uid = user_uid(&user).context("non è un utente locale")?;
    Ok((user, uid))
}

fn service_ok(service: &str) -> Result<()> {
    if !pam::SERVICES.contains(&service) {
        bail!(
            "il telefono può approvare solo questi servizi: {}",
            pam::SERVICES.join(", ")
        );
    }
    Ok(())
}

pub fn run(cmd: ApproveCmd) -> Result<()> {
    match cmd {
        ApproveCmd::Enroll { device } => enroll(&device),
        ApproveCmd::Remove => {
            need_root()?;
            let (user, _) = sudo_user()?;
            let p = keyfile::key_path(Path::new(keyfile::KEY_DIR), &user)
                .map_err(|e| anyhow::anyhow!(e.0))?;
            match std::fs::remove_file(&p) {
                Ok(()) => println!("Chiave rimossa: {}", p.display()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    println!("Nessuna chiave per {user}.")
                }
                Err(e) => return Err(e.into()),
            }
            Ok(())
        }
        ApproveCmd::Status => status(),
        ApproveCmd::Enable { service, helper } => {
            need_root()?;
            service_ok(&service)?;
            pam::helper_is_safe(&helper)
                .map_err(|e| anyhow::anyhow!("{e}: {}", helper.display()))?;
            if service == "polkit-1" {
                polkit_agent_is_setuid()?;
            }
            edit_pam(&service, |t| pam::add_line(t, &helper))?;
            println!(
                "Il telefono ora può approvare `{service}`. La password resta sempre disponibile."
            );
            Ok(())
        }
        ApproveCmd::Disable { service } => {
            need_root()?;
            service_ok(&service)?;
            edit_pam(&service, pam::remove_lines)?;
            println!("`{service}` torna a chiedere solo la password.");
            Ok(())
        }
    }
}

/// polkit gives the helper no item for who asks; it is safe only when the
/// agent helper runs setuid as the user's own process.
fn polkit_agent_is_setuid() -> Result<()> {
    use std::os::unix::fs::MetadataExt;
    for p in [
        "/usr/lib/polkit-1/polkit-agent-helper-1",
        "/usr/libexec/polkit-agent-helper-1",
    ] {
        if let Ok(m) = std::fs::metadata(p) {
            if m.mode() & 0o4000 != 0 && m.uid() == 0 {
                return Ok(());
            }
            bail!(
                "{p} non è setuid root: qui polkit lo esegue come servizio di sistema, quindi l'approvazione dal telefono non può funzionare in modo sicuro per polkit-1"
            );
        }
    }
    bail!("polkit-agent-helper-1 non trovato")
}

/// Rewrite `/etc/pam.d/<service>` atomically, keeping a backup.
fn edit_pam(service: &str, f: impl FnOnce(&str) -> Option<String>) -> Result<()> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let path = Path::new(pam::PAM_DIR).join(service);
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("{} non trovato", path.display()))?;
    let Some(new) = f(&text) else {
        println!("Nessuna modifica necessaria in {}.", path.display());
        return Ok(());
    };
    let backup_dir = Path::new("/etc/konnekta/pam-backup");
    std::fs::create_dir_all(backup_dir)?;
    std::fs::set_permissions(backup_dir, std::fs::Permissions::from_mode(0o755))?;
    let backup = backup_dir.join(format!("{service}.{}", now_secs()));
    std::fs::write(&backup, &text)?;
    let tmp = Path::new(pam::PAM_DIR).join(format!(".{service}.konnekta.tmp"));
    let _ = std::fs::remove_file(&tmp);
    {
        let mut out = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o644)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&tmp)?;
        out.write_all(new.as_bytes())?;
        out.sync_all()?;
    }
    std::fs::rename(&tmp, &path)?;
    println!(
        "Modificato {} (copia di sicurezza in {}).",
        path.display(),
        backup.display()
    );
    Ok(())
}

fn status() -> Result<()> {
    // SAFETY: getuid never fails.
    let me = unsafe { libc::getuid() };
    let user = if me == 0 {
        sudo_user().map(|u| u.0).unwrap_or_else(|_| "root".into())
    } else {
        whoami()?
    };
    let p =
        keyfile::key_path(Path::new(keyfile::KEY_DIR), &user).map_err(|e| anyhow::anyhow!(e.0))?;
    match keyfile::read(&p, 0) {
        Ok(k) => println!(
            "Telefono registrato per {user}: {} ({})",
            k.device_name, k.device_id
        ),
        Err(e) if p.exists() => println!("Chiave di {user} non valida: {e}"),
        Err(_) => println!("Nessun telefono registrato per {user}."),
    }
    for s in pam::SERVICES {
        let on = std::fs::read_to_string(Path::new(pam::PAM_DIR).join(s))
            .map(|t| pam::add_line(&t, Path::new("/x")).is_none())
            .unwrap_or(false);
        println!(
            "  {s:<9} {}",
            if on {
                "approvabile dal telefono"
            } else {
                "solo password"
            }
        );
    }
    Ok(())
}

fn whoami() -> Result<String> {
    std::env::var("USER").context("utente sconosciuto")
}

fn enroll(device: &str) -> Result<()> {
    need_root()?;
    let (user, uid) = sudo_user()?;
    let settings = enroll::Settings {
        key_dir: PathBuf::from(keyfile::KEY_DIR),
        socket: socket_path(uid),
        user: user.clone(),
        uid,
        host: hostname(),
    };
    let mut tty =
        std::io::BufReader::new(std::fs::File::open("/dev/tty").context("serve un terminale")?);
    let mut ask = |prompt: &str| {
        print!("{prompt}");
        std::io::stdout().flush().ok();
        let mut line = String::new();
        tty.read_line(&mut line).map(|_| line)
    };
    let path = enroll::run(&settings, device, &mut ask, &mut |l| println!("{l}"))
        .map_err(anyhow::Error::msg)?;
    println!(
        "Il telefono può ora approvare per {user} ({}).",
        path.display()
    );
    println!("Attiva un servizio con: sudo konnekta approve enable sudo");
    Ok(())
}
