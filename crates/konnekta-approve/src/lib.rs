//! Approve `sudo` and polkit with the phone's fingerprint.
//!
//! * The phone signs each request with a P-256 key that never leaves its
//!   secure hardware (Android Keystore, Secure Enclave), only after a
//!   biometric check.
//! * The public key lives in `/etc/konnekta/approve/<user>.pub`, owned by
//!   root ([`keyfile`]); the daemon, which runs as the user, cannot change it.
//! * `konnekta-approve`, started by `pam_exec`, builds the request itself
//!   (fresh 32-byte nonce, time), asks the phone through the user's daemon
//!   and verifies the signature over its own bytes ([`helper`]). The daemon
//!   only carries messages: a compromised daemon can make PAM fall back to
//!   the password, never forge an approval.
//!
//! [`message`] and [`key`] are portable (the daemon validates requests with
//! them on every OS); the PAM side is Unix only.

#[cfg(unix)]
pub mod client;
#[cfg(unix)]
pub mod enroll;
#[cfg(unix)]
pub mod helper;
pub mod key;
#[cfg(unix)]
pub mod keyfile;
pub mod message;
#[cfg(unix)]
pub mod pam;
#[cfg(unix)]
pub mod setup;
