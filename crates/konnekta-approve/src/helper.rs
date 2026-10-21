//! The PAM side: `pam_exec` runs `konnekta-approve`, which asks the phone
//! and exits 0 only for a valid signature over a request it made itself.
//! Every failure exits non-zero, and PAM asks for the password as usual.

use crate::client::Client;
use crate::key::KeyFile;
use crate::keyfile;
use crate::message::Approval;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// How long the phone has to answer.
pub const WAIT: Duration = Duration::from_secs(25);
const CONNECT: Duration = Duration::from_secs(2);
/// Hard limit for the whole helper, whatever the daemon does.
pub const MAX_TOTAL: Duration = Duration::from_secs(60);

/// PAM items, from the environment `pam_exec` sets.
#[derive(Debug, Clone, Default)]
pub struct PamEnv {
    pub user: String,
    pub service: String,
    pub tty: String,
    pub rhost: String,
    pub ruser: String,
    pub pam_type: String,
    /// Real uid of the helper process.
    pub real_uid: u32,
}

impl PamEnv {
    pub fn from_env() -> Self {
        let v = |k: &str| std::env::var(k).unwrap_or_default();
        Self {
            user: v("PAM_USER"),
            service: v("PAM_SERVICE"),
            tty: v("PAM_TTY"),
            rhost: v("PAM_RHOST"),
            ruser: v("PAM_RUSER"),
            pam_type: v("PAM_TYPE"),
            // SAFETY: getuid never fails.
            real_uid: unsafe { libc::getuid() },
        }
    }
}

/// Finds the uid of a local user name.
pub type UidLookup = Box<dyn Fn(&str) -> Option<u32> + Send>;

/// Where the helper looks. Production values are fixed in [`Settings::system`].
pub struct Settings {
    pub key_dir: PathBuf,
    /// Required owner of the key file (root).
    pub key_owner: u32,
    pub socket: Box<dyn Fn(u32) -> PathBuf + Send>,
    pub host: String,
    pub lookup_uid: UidLookup,
}

impl Settings {
    pub fn system() -> Self {
        Self {
            key_dir: PathBuf::from(keyfile::KEY_DIR),
            key_owner: 0,
            socket: Box::new(crate::client::socket_path),
            host: hostname(),
            lookup_uid: Box::new(user_uid),
        }
    }
}

pub fn hostname() -> String {
    let mut buf = [0u8; 256];
    // SAFETY: buffer and length are valid.
    let r = unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) };
    if r != 0 {
        return String::new();
    }
    let n = buf.iter().position(|b| *b == 0).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..n]).into_owned()
}

/// Uid of a local user from the passwd database.
pub fn user_uid(name: &str) -> Option<u32> {
    let c = std::ffi::CString::new(name).ok()?;
    let mut pw: libc::passwd = unsafe { std::mem::zeroed() };
    let mut buf = vec![0 as libc::c_char; 16 * 1024];
    let mut out: *mut libc::passwd = std::ptr::null_mut();
    // SAFETY: all pointers are valid for the call; the result is checked.
    let r = unsafe { libc::getpwnam_r(c.as_ptr(), &mut pw, buf.as_mut_ptr(), buf.len(), &mut out) };
    (r == 0 && !out.is_null()).then_some(pw.pw_uid)
}

pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn nonce() -> Option<String> {
    let mut b = [0u8; 32];
    ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut b).ok()?;
    Some(hex::encode(b))
}

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Approved,
    /// Not handled: no key, refused request, daemon or phone unavailable,
    /// denied, timeout, bad signature. The message is for the journal.
    Fallback(String),
}

fn fallback<T>(m: impl Into<String>) -> Result<T, Outcome> {
    Err(Outcome::Fallback(m.into()))
}

/// Checks on who asks, before touching the key or the network.
pub fn check_request(env: &PamEnv, user_uid: Option<u32>) -> Result<u32, Outcome> {
    if env.pam_type != "auth" {
        return fallback("not an auth request");
    }
    if env.service == "sshd"
        || env.service.starts_with("login")
        || !crate::pam::SERVICES.contains(&env.service.as_str())
    {
        return fallback(format!("service {} is not approved by phone", env.service));
    }
    if !keyfile::valid_user(&env.user) {
        return fallback("invalid user name");
    }
    let Some(uid) = user_uid else {
        return fallback("not a local user");
    };
    // Another user asks for this user (e.g. sudoers `targetpw`).
    if !env.ruser.is_empty() && env.ruser != env.user {
        return fallback("request made by another user");
    }
    if env.real_uid != 0 && env.real_uid != uid {
        return fallback("request made by another user");
    }
    // polkit gives no item for who asks: the agent helper must run as the user.
    if env.service == "polkit-1" && env.real_uid != uid {
        return fallback("polkit request not from the user's agent");
    }
    Ok(uid)
}

/// The whole approval. `say` prints the one line the user sees.
pub fn run(env: &PamEnv, s: &Settings, say: &mut dyn FnMut(&str)) -> Outcome {
    match run_inner(env, s, say) {
        Ok(()) => Outcome::Approved,
        Err(o) => o,
    }
}

fn run_inner(env: &PamEnv, s: &Settings, say: &mut dyn FnMut(&str)) -> Result<(), Outcome> {
    let started = Instant::now();
    let uid = check_request(env, (s.lookup_uid)(&env.user))?;
    let path =
        keyfile::key_path(&s.key_dir, &env.user).map_err(|e| Outcome::Fallback(e.0.into()))?;
    if !path.exists() {
        return fallback("no key enrolled");
    }
    let key: KeyFile =
        keyfile::read(&path, s.key_owner).map_err(|e| Outcome::Fallback(e.0.into()))?;
    let approval = Approval {
        host: s.host.clone(),
        user: env.user.clone(),
        service: env.service.clone(),
        tty: env.tty.clone(),
        rhost: env.rhost.clone(),
        time: now_secs(),
        nonce: nonce().ok_or(Outcome::Fallback("no randomness".into()))?,
    };
    let message = approval
        .message()
        .map_err(|e| Outcome::Fallback(e.to_string()))?;

    let mut c =
        Client::connect(&(s.socket)(uid), uid, CONNECT).map_err(|e| Outcome::Fallback(e.0))?;
    let r = c
        .approve(
            &key.device_id,
            "request",
            json!({
                "host": approval.host, "user": approval.user, "service": approval.service,
                "tty": approval.tty, "rhost": approval.rhost, "time": approval.time,
                "nonce": approval.nonce, "timeout": WAIT.as_secs(),
            }),
        )
        .map_err(|e| Outcome::Fallback(e.0))?;
    let id = r
        .get("id")
        .and_then(Value::as_str)
        .ok_or(Outcome::Fallback("no request id".into()))?
        .to_owned();
    let device = if key.device_name.is_empty() {
        "il telefono".to_owned()
    } else {
        key.device_name.clone()
    };
    say(&if approval.tty.is_empty() {
        format!("Approva su {device}, oppure attendi la richiesta della password.")
    } else {
        format!(
            "Approva su {device} per il terminale {}, oppure attendi la richiesta della password.",
            approval.tty
        )
    });

    let left = MAX_TOTAL.saturating_sub(started.elapsed());
    c.set_timeout((WAIT + Duration::from_secs(5)).min(left));
    let answer = c
        .approve(&key.device_id, "wait", json!({ "id": id }))
        .map_err(|e| Outcome::Fallback(e.0))?;
    if answer.get("result").and_then(Value::as_str) != Some("approved") {
        return fallback(format!(
            "phone answered {}",
            answer
                .get("result")
                .and_then(Value::as_str)
                .unwrap_or("nothing")
        ));
    }
    if started.elapsed() > MAX_TOTAL {
        return fallback("too late");
    }
    let sig = answer
        .get("signature")
        .and_then(Value::as_str)
        .and_then(|s| B64.decode(s).ok())
        .ok_or(Outcome::Fallback("no signature".into()))?;
    // Verify over the bytes built here, never over anything sent back.
    if !key.key.verify(&message, &sig) {
        return fallback("bad signature");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env() -> PamEnv {
        PamEnv {
            user: "u".into(),
            service: "sudo".into(),
            pam_type: "auth".into(),
            real_uid: 0,
            ..Default::default()
        }
    }

    #[test]
    fn who_may_ask() {
        assert_eq!(check_request(&env(), Some(1000)), Ok(1000));
        let with = |f: fn(&mut PamEnv)| {
            let mut e = env();
            f(&mut e);
            check_request(&e, Some(1000))
        };
        assert!(with(|e| e.service = "sshd".into()).is_err());
        assert!(with(|e| e.service = "login".into()).is_err());
        assert!(
            with(|e| e.service = "su".into()).is_err(),
            "only listed services"
        );
        assert!(with(|e| e.pam_type = "account".into()).is_err());
        assert!(with(|e| e.ruser = "mallory".into()).is_err());
        assert!(
            with(|e| e.real_uid = 1001).is_err(),
            "another user's process"
        );
        assert!(
            with(|e| e.real_uid = 1000).is_ok(),
            "the user's own process"
        );
        assert!(
            with(|e| e.service = "polkit-1".into()).is_err(),
            "polkit agent as root"
        );
        assert!(
            with(|e| {
                e.service = "polkit-1".into();
                e.real_uid = 1000;
            })
            .is_ok()
        );
        assert!(with(|e| e.user = "../root".into()).is_err());
        assert!(check_request(&env(), None).is_err(), "not a local user");
    }
}
