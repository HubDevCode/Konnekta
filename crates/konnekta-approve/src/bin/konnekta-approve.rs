//! PAM helper: `auth sufficient pam_exec.so quiet stdout <path>/konnekta-approve`.
//!
//! No flags and no environment except the PAM items: every path is fixed.
//! Exit 0 only for a valid phone signature; anything else lets PAM ask for
//! the password.

#[cfg(unix)]
fn main() {
    use konnekta_approve::helper::{Outcome, PamEnv, Settings, run};
    let env = PamEnv::from_env();
    let outcome = run(&env, &Settings::system(), &mut |line| println!("{line}"));
    match outcome {
        Outcome::Approved => std::process::exit(0),
        Outcome::Fallback(reason) => {
            // pam_exec forwards stderr to the journal only.
            eprintln!("konnekta-approve: {reason}");
            std::process::exit(1)
        }
    }
}

#[cfg(not(unix))]
fn main() {
    std::process::exit(1);
}
