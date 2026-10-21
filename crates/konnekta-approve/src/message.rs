//! The exact bytes the phone signs.
//!
//! The helper, the daemon and the phone build them from the same fields
//! with the same rules, and the helper always rebuilds them from *its own*
//! fields before verifying: nothing it gets back from the daemon or the
//! phone is trusted.
//!
//! Approval (8 lines, each ending with `\n`):
//!
//! ```text
//! konnekta-approve-v1
//! host=<host name>
//! user=<user name>
//! service=<PAM service>
//! tty=<PAM_TTY or empty>
//! rhost=<PAM_RHOST or empty>
//! time=<Unix seconds>
//! nonce=<64 lowercase hex digits>
//! ```
//!
//! Enrollment (6 lines): `konnekta-approve-enroll-v1`, `host`, `user`,
//! `key` (SHA-256 of the public key, 64 hex), `time`, `nonce`. The first
//! line names version and purpose, so an enrollment signature is never a
//! valid approval and vice versa.

use std::fmt;

pub const APPROVE_V1: &str = "konnekta-approve-v1";
pub const ENROLL_V1: &str = "konnekta-approve-enroll-v1";
pub const MAX_VALUE: usize = 256;
/// Accepted signed times: 1 ..= 2^40 seconds.
pub const MAX_TIME: u64 = 1 << 40;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldError {
    Empty(&'static str),
    TooLong(&'static str),
    Control(&'static str),
    BadNonce,
    BadKey,
    BadTime,
}

impl fmt::Display for FieldError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty(k) => write!(f, "{k} is empty"),
            Self::TooLong(k) => write!(f, "{k} is too long"),
            Self::Control(k) => write!(f, "{k} has a control character"),
            Self::BadNonce => write!(f, "nonce must be 64 lowercase hex digits"),
            Self::BadKey => write!(f, "key must be 64 lowercase hex digits"),
            Self::BadTime => write!(f, "time out of range"),
        }
    }
}

impl std::error::Error for FieldError {}

/// Control characters, including the bidi controls that reorder text.
pub fn is_control(c: char) -> bool {
    c.is_control()
        || matches!(c, '\u{061C}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

fn check(name: &'static str, v: &str, required: bool) -> Result<(), FieldError> {
    if required && v.is_empty() {
        return Err(FieldError::Empty(name));
    }
    if v.len() > MAX_VALUE {
        return Err(FieldError::TooLong(name));
    }
    if v.chars().any(is_control) {
        return Err(FieldError::Control(name));
    }
    Ok(())
}

fn is_hex64(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn check_time(t: u64) -> Result<(), FieldError> {
    if (1..=MAX_TIME).contains(&t) {
        Ok(())
    } else {
        Err(FieldError::BadTime)
    }
}

/// What a `sudo` / polkit approval is about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Approval {
    pub host: String,
    pub user: String,
    pub service: String,
    pub tty: String,
    pub rhost: String,
    pub time: u64,
    pub nonce: String,
}

impl Approval {
    pub fn validate(&self) -> Result<(), FieldError> {
        check("host", &self.host, true)?;
        check("user", &self.user, true)?;
        check("service", &self.service, true)?;
        check("tty", &self.tty, false)?;
        check("rhost", &self.rhost, false)?;
        check_time(self.time)?;
        if !is_hex64(&self.nonce) {
            return Err(FieldError::BadNonce);
        }
        Ok(())
    }

    /// The signed bytes; fails if a field breaks a rule.
    pub fn message(&self) -> Result<Vec<u8>, FieldError> {
        self.validate()?;
        Ok(format!(
            "{APPROVE_V1}\nhost={}\nuser={}\nservice={}\ntty={}\nrhost={}\ntime={}\nnonce={}\n",
            self.host, self.user, self.service, self.tty, self.rhost, self.time, self.nonce
        )
        .into_bytes())
    }
}

/// Registration of the phone's key for a user of this computer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Enrollment {
    pub host: String,
    pub user: String,
    /// SHA-256 of the public key (SubjectPublicKeyInfo DER), 64 hex.
    pub key: String,
    pub time: u64,
    pub nonce: String,
}

impl Enrollment {
    pub fn validate(&self) -> Result<(), FieldError> {
        check("host", &self.host, true)?;
        check("user", &self.user, true)?;
        check_time(self.time)?;
        if !is_hex64(&self.key) {
            return Err(FieldError::BadKey);
        }
        if !is_hex64(&self.nonce) {
            return Err(FieldError::BadNonce);
        }
        Ok(())
    }

    pub fn message(&self) -> Result<Vec<u8>, FieldError> {
        self.validate()?;
        Ok(format!(
            "{ENROLL_V1}\nhost={}\nuser={}\nkey={}\ntime={}\nnonce={}\n",
            self.host, self.user, self.key, self.time, self.nonce
        )
        .into_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approval() -> Approval {
        Approval {
            host: "arch".into(),
            user: "naitsirk".into(),
            service: "sudo".into(),
            tty: "/dev/pts/3".into(),
            rhost: String::new(),
            time: 1_760_000_000,
            nonce: "ab".repeat(32),
        }
    }

    /// Test vector shared with the phone apps: they must build these bytes.
    #[test]
    fn approval_bytes() {
        let m = approval().message().unwrap();
        assert_eq!(
            String::from_utf8(m).unwrap(),
            format!(
                "konnekta-approve-v1\nhost=arch\nuser=naitsirk\nservice=sudo\ntty=/dev/pts/3\nrhost=\ntime=1760000000\nnonce={}\n",
                "ab".repeat(32)
            )
        );
    }

    #[test]
    fn enrollment_bytes() {
        let e = Enrollment {
            host: "arch".into(),
            user: "u".into(),
            key: "0".repeat(64),
            time: 5,
            nonce: "f".repeat(64),
        };
        assert_eq!(
            String::from_utf8(e.message().unwrap()).unwrap(),
            format!(
                "konnekta-approve-enroll-v1\nhost=arch\nuser=u\nkey={}\ntime=5\nnonce={}\n",
                "0".repeat(64),
                "f".repeat(64)
            )
        );
    }

    #[test]
    fn a_value_can_never_add_a_line() {
        let mut a = approval();
        a.tty = "x\nservice=login".into();
        assert_eq!(a.message(), Err(FieldError::Control("tty")));
        let mut a = approval();
        a.host = "evil\u{202E}".into();
        assert_eq!(a.message(), Err(FieldError::Control("host")));
        let mut a = approval();
        a.nonce = "AB".repeat(32);
        assert_eq!(a.message(), Err(FieldError::BadNonce));
        let mut a = approval();
        a.service = String::new();
        assert_eq!(a.message(), Err(FieldError::Empty("service")));
        let mut a = approval();
        a.time = 0;
        assert_eq!(a.message(), Err(FieldError::BadTime));
        let mut a = approval();
        a.user = "u".repeat(MAX_VALUE + 1);
        assert_eq!(a.message(), Err(FieldError::TooLong("user")));
    }
}
