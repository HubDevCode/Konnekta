//! `konnekta` — daemon and command line client.

mod ipc;

/// Like `println!` but exits quietly when stdout is closed (`| head`).
macro_rules! println {
    ($($t:tt)*) => {{
        use std::io::Write as _;
        if writeln!(std::io::stdout(), $($t)*).is_err() {
            std::process::exit(0);
        }
    }};
}

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use konnekta_core::{Config, Konnekta};
use serde_json::{Value, json};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "konnekta", version, about = "Connect your phone and computer — securely")]
struct Cli {
    /// Configuration / key directory.
    #[arg(long, env = "KONNEKTA_DATA_DIR", global = true)]
    data_dir: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run the background service.
    Daemon {
        /// Device name shown to other devices.
        #[arg(long)]
        name: Option<String>,
        /// Do not broadcast our presence (manual `connect` still works).
        #[arg(long)]
        no_announce: bool,
        #[arg(short, long)]
        verbose: bool,
    },
    /// Show this device's identity and certificate fingerprint.
    Info,
    /// List known and nearby devices.
    Devices,
    /// Ask a device at a known IP address to connect.
    Connect { address: String },
    /// Request pairing (compare the code shown on both screens!).
    Pair { device: String },
    /// Accept a pairing request.
    Accept { device: String },
    /// Reject a pairing request.
    Reject { device: String },
    /// Forget a paired device.
    Unpair { device: String },
    /// Show granted permissions.
    Perms { device: String },
    /// Grant permissions (e.g. `grant phone remoteInput commands`).
    Grant { device: String, perms: Vec<String> },
    /// Revoke permissions.
    Revoke { device: String, perms: Vec<String> },
    /// Send a ping.
    Ping { device: String, message: Option<String> },
    /// Send files.
    Send { device: String, files: Vec<PathBuf> },
    /// Send text (lands in the phone clipboard / share sheet).
    Text { device: String, text: String },
    /// Make the phone ring.
    Ring { device: String },
    /// List plugins.
    Plugins,
    /// Enable a plugin globally.
    Enable { plugin: String },
    /// Disable a plugin globally.
    Disable { plugin: String },
    /// Call any plugin action.
    Action {
        plugin: String,
        action: String,
        #[arg(long)]
        device: Option<String>,
        /// JSON arguments.
        #[arg(long, default_value = "null")]
        args: String,
    },
    /// Stream events as JSON lines.
    Events,
}

fn data_dir(cli: &Cli) -> PathBuf {
    cli.data_dir.clone().unwrap_or_else(Config::default_data_dir)
}

async fn run_daemon(dir: PathBuf, name: Option<String>, no_announce: bool) -> Result<()> {
    std::fs::create_dir_all(&dir)?;
    let cfg_path = dir.join("config.json");
    let mut cfg = Config::load(&cfg_path)?;
    if let Some(n) = name {
        cfg.device_name = n;
    }
    if no_announce {
        cfg.announce = false;
    }
    cfg.save(&cfg_path)?;
    let platform = konnekta_platform::Platform::native();
    let k = Konnekta::builder(cfg)
        .data_dir(&dir)
        .plugins(konnekta_plugins::all(platform))
        .build()?;
    k.start().await?;
    println!("Konnekta {} — device id {}", env!("CARGO_PKG_VERSION"), k.device_id());
    println!("certificate fingerprint {}", k.fingerprint());

    let mut events = k.subscribe();
    tokio::spawn(async move {
        while let Ok(ev) = events.recv().await {
            if let konnekta_core::Event::PairRequested { device_name, verification_key, device_id } = &ev {
                println!(
                    "\n>>> Pairing request from \"{device_name}\" — verification code {verification_key}\n    \
                     compare it with the code on the other device, then run: konnekta accept {device_id}\n"
                );
            }
            if let konnekta_core::Event::SecurityWarning { message, .. } = &ev {
                eprintln!("[security] {message}");
            }
        }
    });

    let endpoint = ipc::endpoint(&dir);
    let server = tokio::spawn(ipc::listen(endpoint.clone(), k.clone()));
    tokio::select! {
        r = server => { r??; }
        _ = tokio::signal::ctrl_c() => {}
    }
    k.shutdown();
    #[cfg(unix)]
    let _ = std::fs::remove_file(endpoint);
    Ok(())
}

/// Resolve a device by id, id prefix or (case-insensitive) name.
async fn resolve(c: &mut ipc::Client, q: &str) -> Result<String> {
    let devs = c.call("devices", Value::Null).await?;
    let devs = devs.as_array().cloned().unwrap_or_default();
    let ql = q.to_lowercase();
    let matches: Vec<&Value> = devs
        .iter()
        .filter(|d| {
            let id = d["id"].as_str().unwrap_or_default();
            let name = d["name"].as_str().unwrap_or_default().to_lowercase();
            id == q || id.starts_with(q) || name == ql
        })
        .collect();
    match matches.as_slice() {
        [d] => Ok(d["id"].as_str().unwrap_or_default().to_owned()),
        [] => bail!("no device matches '{q}' (see `konnekta devices`)"),
        _ => bail!("'{q}' is ambiguous"),
    }
}

fn print_devices(v: &Value) {
    let devs = v.as_array().cloned().unwrap_or_default();
    if devs.is_empty() {
        println!("No devices. Make sure both devices are on the same network, or use `konnekta connect <ip>`.");
    }
    for d in devs {
        println!(
            "{:<24} {:<8} {:<10} {:<16} {}",
            d["name"].as_str().unwrap_or_default(),
            d["deviceType"].as_str().unwrap_or_default(),
            if d["paired"].as_bool() == Some(true) { "paired" } else { d["pairState"].as_str().unwrap_or_default() },
            if d["connected"].as_bool() == Some(true) { d["address"].as_str().unwrap_or("online") } else { "offline" },
            d["id"].as_str().unwrap_or_default(),
        );
    }
}

async fn action(c: &mut ipc::Client, dev: Option<String>, plugin: &str, action: &str, args: Value) -> Result<Value> {
    c.call("pluginAction", json!({ "deviceId": dev, "plugin": plugin, "action": action, "args": args })).await
}

fn print(v: &Value) {
    if !v.is_null() {
        println!("{}", serde_json::to_string_pretty(v).unwrap_or_default());
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let dir = data_dir(&cli);
    if let Cmd::Daemon { name, no_announce, verbose } = cli.cmd {
        let filter = if verbose { "konnekta=debug,info" } else { "info" };
        tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| filter.into()))
            .init();
        return run_daemon(dir, name, no_announce).await;
    }
    let mut c = ipc::Client::connect(&ipc::endpoint(&dir)).await?;
    match cli.cmd {
        Cmd::Daemon { .. } => unreachable!(),
        Cmd::Info => print(&c.call("info", Value::Null).await?),
        Cmd::Devices => print_devices(&c.call("devices", Value::Null).await?),
        Cmd::Connect { address } => {
            c.call("connect", json!({ "address": address })).await?;
            println!("Request sent. The device should appear in `konnekta devices`.");
        }
        Cmd::Pair { device } => {
            let id = resolve(&mut c, &device).await?;
            let r = c.call("pair", json!({ "deviceId": id })).await?;
            println!(
                "Pairing requested. Verification code: {}\nAccept on the other device only if it shows the same code.",
                r["verificationKey"].as_str().unwrap_or_default()
            );
        }
        Cmd::Accept { device } => {
            let id = resolve(&mut c, &device).await?;
            c.call("acceptPair", json!({ "deviceId": id })).await?;
            println!("Paired.");
        }
        Cmd::Reject { device } => {
            let id = resolve(&mut c, &device).await?;
            c.call("rejectPair", json!({ "deviceId": id })).await?;
        }
        Cmd::Unpair { device } => {
            let id = resolve(&mut c, &device).await?;
            c.call("unpair", json!({ "deviceId": id })).await?;
            println!("Unpaired.");
        }
        Cmd::Perms { device } => {
            let id = resolve(&mut c, &device).await?;
            let d = c.call("device", json!({ "deviceId": id })).await?;
            let granted: Vec<String> =
                serde_json::from_value(d["permissions"].clone()).unwrap_or_default();
            for p in c.call("permissions", Value::Null).await?.as_array().cloned().unwrap_or_default() {
                let pid = p["id"].as_str().unwrap_or_default();
                println!(
                    "[{}] {:<14} {}{}",
                    if granted.iter().any(|g| g == pid) { "x" } else { " " },
                    pid,
                    p["label"].as_str().unwrap_or_default(),
                    if p["sensitive"].as_bool() == Some(true) { "  (sensitive)" } else { "" }
                );
            }
        }
        Cmd::Grant { device, perms } | Cmd::Revoke { device, perms } if perms.is_empty() => {
            bail!("no permissions given for {device}")
        }
        Cmd::Grant { device, perms } => {
            let id = resolve(&mut c, &device).await?;
            for p in perms {
                c.call("setPermission", json!({ "deviceId": id, "permission": p, "granted": true })).await?;
            }
        }
        Cmd::Revoke { device, perms } => {
            let id = resolve(&mut c, &device).await?;
            for p in perms {
                c.call("setPermission", json!({ "deviceId": id, "permission": p, "granted": false })).await?;
            }
        }
        Cmd::Ping { device, message } => {
            let id = resolve(&mut c, &device).await?;
            action(&mut c, Some(id), "ping", "send", json!({ "message": message })).await?;
        }
        Cmd::Send { device, files } => {
            let id = resolve(&mut c, &device).await?;
            let files: Vec<PathBuf> = files
                .into_iter()
                .map(|f| std::fs::canonicalize(&f).with_context(|| format!("{}", f.display())))
                .collect::<Result<_>>()?;
            action(&mut c, Some(id), "share", "sendFiles", json!({ "paths": files })).await?;
            println!("Transfer started (follow it with `konnekta events`).");
        }
        Cmd::Text { device, text } => {
            let id = resolve(&mut c, &device).await?;
            action(&mut c, Some(id), "share", "sendText", json!({ "text": text })).await?;
        }
        Cmd::Ring { device } => {
            let id = resolve(&mut c, &device).await?;
            action(&mut c, Some(id), "findmyphone", "ring", Value::Null).await?;
        }
        Cmd::Plugins => {
            for p in c.call("plugins", Value::Null).await?.as_array().cloned().unwrap_or_default() {
                println!(
                    "[{}] {:<14} {:<10} {}",
                    if p["enabled"].as_bool() == Some(true) { "x" } else { " " },
                    p["id"].as_str().unwrap_or_default(),
                    p["origin"].as_str().unwrap_or_default(),
                    p["description"].as_str().unwrap_or_default()
                );
            }
        }
        Cmd::Enable { plugin } => {
            c.call("setPluginEnabled", json!({ "plugin": plugin, "enabled": true })).await?;
        }
        Cmd::Disable { plugin } => {
            c.call("setPluginEnabled", json!({ "plugin": plugin, "enabled": false })).await?;
        }
        Cmd::Action { plugin, action: a, device, args } => {
            let dev = match device {
                Some(d) => Some(resolve(&mut c, &d).await?),
                None => None,
            };
            let args: Value = serde_json::from_str(&args).context("--args must be JSON")?;
            print(&action(&mut c, dev, &plugin, &a, args).await?);
        }
        Cmd::Events => {
            c.subscribe(|v| println!("{v}")).await?;
        }
    }
    Ok(())
}
