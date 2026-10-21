//! `konnekta` — daemon and command line client.

use konnekta_core::ipc;

/// Like `println!` but exits quietly when stdout is closed (`| head`).
macro_rules! println {
    ($($t:tt)*) => {{
        use std::io::Write as _;
        if writeln!(std::io::stdout(), $($t)*).is_err() {
            std::process::exit(0);
        }
    }};
}

#[cfg(unix)]
mod approve;
mod waybar;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use konnekta_core::{Config, Konnekta};
use serde_json::{Value, json};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "konnekta",
    version,
    about = "Connect your phone and computer — securely"
)]
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
        /// Do not broadcast our presence (paired devices are still dialed).
        #[arg(long)]
        no_announce: bool,
        /// Also let KDE Connect / GSConnect apps connect (saved).
        #[arg(long)]
        kde_connect: bool,
        /// Also let Flux apps connect (saved).
        #[arg(long)]
        flux: bool,
        #[arg(short, long)]
        verbose: bool,
    },
    /// Show this device's identity and certificate fingerprint.
    Info,
    /// List known and nearby devices.
    Devices,
    /// Devices seen on the network that are not connected.
    Discover,
    /// Connect to a device by address (`ip`, `ip:port` or host name).
    Connect { address: String },
    /// Request pairing, then confirm the 16-character key.
    Pair { device: String },
    /// Accept a request, or confirm a pairing; KEY = the key you compared.
    Accept { device: String, key: Vec<String> },
    /// Reject an open pairing.
    Reject { device: String, key: Vec<String> },
    /// Set the addresses dialed when a paired device is away (Tailscale...).
    Addresses {
        device: String,
        addresses: Vec<String>,
    },
    /// Forget a paired device.
    Unpair { device: String },
    /// Show granted permissions.
    Perms { device: String },
    /// Grant permissions (e.g. `grant phone remoteInput commands`).
    Grant { device: String, perms: Vec<String> },
    /// Revoke permissions.
    Revoke { device: String, perms: Vec<String> },
    /// Send a ping.
    Ping {
        device: String,
        message: Option<String>,
    },
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
    /// Status for a Waybar `custom` module (JSON lines, keeps running).
    Waybar,
    /// Check what the media features need on this computer and print the
    /// commands that install it (nothing is run).
    Setup {
        /// Print the report as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Approve sudo / polkit with the phone's fingerprint.
    #[cfg(unix)]
    Approve {
        #[command(subcommand)]
        cmd: approve::ApproveCmd,
    },
}

fn data_dir(cli: &Cli) -> PathBuf {
    cli.data_dir
        .clone()
        .unwrap_or_else(Config::default_data_dir)
}

async fn run_daemon(
    dir: PathBuf,
    name: Option<String>,
    no_announce: bool,
    adapters: &[konnekta_core::proto::Protocol],
) -> Result<()> {
    std::fs::create_dir_all(&dir)?;
    let cfg_path = dir.join("config.json");
    let mut cfg = Config::load(&cfg_path)?;
    if let Some(n) = name {
        cfg.device_name = n;
    }
    if no_announce {
        cfg.announce = false;
    }
    for p in adapters {
        if let Some(a) = cfg.adapter_mut(*p) {
            a.enabled = true;
        }
    }
    cfg.save(&cfg_path)?;
    let platform = konnekta_platform::Platform::native();
    let k = Konnekta::builder(cfg)
        .data_dir(&dir)
        .plugins(konnekta_plugins::all(platform))
        .build()?;
    k.start().await?;
    println!(
        "Konnekta {} — device id {}",
        env!("CARGO_PKG_VERSION"),
        k.device_id()
    );
    println!("certificate fingerprint {}", k.fingerprint());

    let mut events = k.subscribe();
    tokio::spawn(async move {
        while let Ok(ev) = events.recv().await {
            if let konnekta_core::Event::PairRequested {
                device_name,
                verification_key,
                device_id,
            } = &ev
            {
                let key = konnekta_crypto::format_key(verification_key);
                println!(
                    "\n>>> Pairing request from \"{device_name}\" — key {key}\n    \
                     compare it with the key on the other device, then run: konnekta accept {device_id} {key}\n"
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
        println!(
            "No devices. Make sure both devices are on the same network, or use `konnekta connect <ip>`."
        );
    }
    for d in devs {
        println!(
            "{:<24} {:<8} {:<10} {:<16} {}",
            d["name"].as_str().unwrap_or_default(),
            d["deviceType"].as_str().unwrap_or_default(),
            if d["paired"].as_bool() == Some(true) {
                "paired"
            } else {
                d["pairState"].as_str().unwrap_or_default()
            },
            if d["connected"].as_bool() == Some(true) {
                d["address"].as_str().unwrap_or("online")
            } else {
                "offline"
            },
            d["id"].as_str().unwrap_or_default(),
        );
    }
}

async fn device_name(c: &mut ipc::Client, id: &str) -> String {
    c.call("device", json!({ "deviceId": id }))
        .await
        .ok()
        .and_then(|d| d["name"].as_str().map(str::to_owned))
        .unwrap_or_else(|| id.to_owned())
}

/// Ask the user to compare the key; only an explicit "y" pins the device.
async fn confirm_key(
    c: &mut ipc::Client,
    id: &str,
    name: &str,
    key: &str,
    after_accept: bool,
) -> Result<()> {
    use std::io::{BufRead, IsTerminal, Write};
    let shown = konnekta_crypto::format_key(key);
    if !std::io::stdin().is_terminal() {
        println!("Compare the key, then run: konnekta accept {id} {shown}");
        return Ok(());
    }
    if after_accept {
        println!("{name} accepted.");
    }
    print!("Does {name} show {shown}? [y/N] ");
    std::io::stdout().flush().ok();
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line)?;
    if line.trim().eq_ignore_ascii_case("y") {
        c.call("acceptPair", json!({ "deviceId": id, "key": key }))
            .await?;
        println!("Paired with {name} ({id}).");
    } else {
        c.call("rejectPair", json!({ "deviceId": id, "key": key }))
            .await?;
        println!("Rejected: the keys must match.");
    }
    Ok(())
}

async fn action(
    c: &mut ipc::Client,
    dev: Option<String>,
    plugin: &str,
    action: &str,
    args: Value,
) -> Result<Value> {
    c.call(
        "pluginAction",
        json!({ "deviceId": dev, "plugin": plugin, "action": action, "args": args }),
    )
    .await
}

fn print(v: &Value) {
    if !v.is_null() {
        println!("{}", serde_json::to_string_pretty(v).unwrap_or_default());
    }
}

fn setup_report(json: bool) {
    use konnekta_media::setup::{self, Status};
    let r = setup::check();
    if json {
        println!("{}", serde_json::to_string_pretty(&r).unwrap_or_default());
        return;
    }
    for f in &r.features {
        println!("{} {}", if f.ok { "✓" } else { "✗" }, f.id);
        for q in f.requirements.iter().filter(|q| q.status != Status::Ok) {
            let state = match q.status {
                Status::Missing => "missing",
                Status::NotLoaded => "not loaded",
                Status::Unsupported => "unsupported",
                Status::Ok => "ok",
            };
            println!("    {} ({state}): {}", q.id, q.purpose);
        }
    }
    if r.all_commands.is_empty() {
        println!("\nEverything is ready.");
    } else {
        println!("\nRun these commands (as yourself; sudo asks for the password):\n");
        for c in &r.all_commands {
            println!("  {c}");
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let dir = data_dir(&cli);
    if let Cmd::Daemon {
        name,
        no_announce,
        kde_connect,
        flux,
        verbose,
    } = cli.cmd
    {
        let filter = if verbose {
            "konnekta=debug,info"
        } else {
            "info"
        };
        tracing_subscriber::fmt()
            .with_env_filter(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| filter.into()),
            )
            .init();
        use konnekta_core::proto::Protocol;
        let adapters: Vec<Protocol> = [(kde_connect, Protocol::KdeConnect), (flux, Protocol::Flux)]
            .into_iter()
            .filter_map(|(on, p)| on.then_some(p))
            .collect();
        return run_daemon(dir, name, no_announce, &adapters).await;
    }
    if let Cmd::Waybar = cli.cmd {
        return waybar::run(dir).await;
    }
    if let Cmd::Setup { json } = cli.cmd {
        setup_report(json);
        return Ok(());
    }
    // Runs as root through sudo: it finds the user's daemon by uid itself.
    #[cfg(unix)]
    if let Cmd::Approve { cmd } = cli.cmd {
        return tokio::task::spawn_blocking(move || approve::run(cmd)).await?;
    }
    let mut c = ipc::Client::connect(&ipc::endpoint(&dir)).await?;
    match cli.cmd {
        Cmd::Daemon { .. } | Cmd::Waybar | Cmd::Setup { .. } => unreachable!(),
        #[cfg(unix)]
        Cmd::Approve { .. } => unreachable!(),
        Cmd::Info => print(&c.call("info", Value::Null).await?),
        Cmd::Devices => print_devices(&c.call("devices", Value::Null).await?),
        Cmd::Connect { address } => {
            c.call("connect", json!({ "address": address })).await?;
            println!("Request sent. The device should appear in `konnekta devices`.");
        }
        Cmd::Discover => {
            for d in c
                .call("discovered", Value::Null)
                .await?
                .as_array()
                .cloned()
                .unwrap_or_default()
            {
                println!(
                    "{:<24} {:<8} {:<12} {:<22} {}",
                    d["name"].as_str().unwrap_or_default(),
                    d["deviceType"].as_str().unwrap_or_default(),
                    d["protocol"].as_str().unwrap_or_default(),
                    d["address"].as_str().unwrap_or_default(),
                    d["id"].as_str().unwrap_or_default(),
                );
            }
        }
        Cmd::Pair { device } => {
            let id = resolve(&mut c, &device).await?;
            let name = device_name(&mut c, &id).await;
            // Listen before asking, so the answer cannot be missed.
            let events = ipc::Client::connect(&ipc::endpoint(&dir)).await?;
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
            tokio::spawn(events.subscribe(move |v| {
                let _ = tx.send(v);
            }));
            let r = c.call("pair", json!({ "deviceId": id })).await?;
            let key = r["verificationKey"].as_str().unwrap_or_default().to_owned();
            println!("{name}  {id}");
            println!("Key: {}", konnekta_crypto::format_key(&key));
            println!("Accept on {name} only if it shows the same key. Waiting…");
            let answer = tokio::time::timeout(std::time::Duration::from_secs(35), async {
                while let Some(v) = rx.recv().await {
                    if v["deviceId"].as_str() != Some(id.as_str()) {
                        continue;
                    }
                    match v["event"].as_str() {
                        Some("pairConfirm") => return Some(true),
                        Some("pairRejected") => return Some(false),
                        _ => {}
                    }
                }
                None
            })
            .await
            .ok()
            .flatten();
            match answer {
                Some(true) => confirm_key(&mut c, &id, &name, &key, true).await?,
                Some(false) => bail!("{name} rejected the pairing"),
                None => bail!("no answer from {name} within 30 seconds"),
            }
        }
        Cmd::Accept { device, key } => {
            let id = resolve(&mut c, &device).await?;
            let name = device_name(&mut c, &id).await;
            let given = key.join("");
            if given.is_empty() {
                let d = c.call("device", json!({ "deviceId": id })).await?;
                let open = d["pairKey"]
                    .as_str()
                    .context("no open pairing with this device")?
                    .to_owned();
                confirm_key(&mut c, &id, &name, &open, false).await?;
            } else {
                c.call("acceptPair", json!({ "deviceId": id, "key": given }))
                    .await?;
                println!("Paired with {name} ({id}).");
            }
        }
        Cmd::Reject { device, key } => {
            let id = resolve(&mut c, &device).await?;
            let key = key.join("");
            c.call(
                "rejectPair",
                json!({ "deviceId": id, "key": (!key.is_empty()).then_some(key) }),
            )
            .await?;
            println!("Rejected.");
        }
        Cmd::Addresses { device, addresses } => {
            let id = resolve(&mut c, &device).await?;
            c.call(
                "setExtraAddresses",
                json!({ "deviceId": id, "addresses": addresses }),
            )
            .await?;
            let d = c.call("device", json!({ "deviceId": id })).await?;
            println!("Extra addresses: {}", d["extraAddresses"]);
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
            for p in c
                .call("permissions", Value::Null)
                .await?
                .as_array()
                .cloned()
                .unwrap_or_default()
            {
                let pid = p["id"].as_str().unwrap_or_default();
                println!(
                    "[{}] {:<14} {}{}",
                    if granted.iter().any(|g| g == pid) {
                        "x"
                    } else {
                        " "
                    },
                    pid,
                    p["label"].as_str().unwrap_or_default(),
                    if p["sensitive"].as_bool() == Some(true) {
                        "  (sensitive)"
                    } else {
                        ""
                    }
                );
            }
        }
        Cmd::Grant { device, perms } | Cmd::Revoke { device, perms } if perms.is_empty() => {
            bail!("no permissions given for {device}")
        }
        Cmd::Grant { device, perms } => {
            let id = resolve(&mut c, &device).await?;
            for p in perms {
                c.call(
                    "setPermission",
                    json!({ "deviceId": id, "permission": p, "granted": true }),
                )
                .await?;
            }
        }
        Cmd::Revoke { device, perms } => {
            let id = resolve(&mut c, &device).await?;
            for p in perms {
                c.call(
                    "setPermission",
                    json!({ "deviceId": id, "permission": p, "granted": false }),
                )
                .await?;
            }
        }
        Cmd::Ping { device, message } => {
            let id = resolve(&mut c, &device).await?;
            action(
                &mut c,
                Some(id),
                "ping",
                "send",
                json!({ "message": message }),
            )
            .await?;
        }
        Cmd::Send { device, files } => {
            let id = resolve(&mut c, &device).await?;
            let files: Vec<PathBuf> = files
                .into_iter()
                .map(|f| std::fs::canonicalize(&f).with_context(|| format!("{}", f.display())))
                .collect::<Result<_>>()?;
            action(
                &mut c,
                Some(id),
                "share",
                "sendFiles",
                json!({ "paths": files }),
            )
            .await?;
            println!("Transfer started (follow it with `konnekta events`).");
        }
        Cmd::Text { device, text } => {
            let id = resolve(&mut c, &device).await?;
            action(
                &mut c,
                Some(id),
                "share",
                "sendText",
                json!({ "text": text }),
            )
            .await?;
        }
        Cmd::Ring { device } => {
            let id = resolve(&mut c, &device).await?;
            action(&mut c, Some(id), "finddevice", "ring", Value::Null).await?;
        }
        Cmd::Plugins => {
            for p in c
                .call("plugins", Value::Null)
                .await?
                .as_array()
                .cloned()
                .unwrap_or_default()
            {
                println!(
                    "[{}] {:<14} {:<16} {}",
                    if p["enabled"].as_bool() == Some(true) {
                        "x"
                    } else {
                        " "
                    },
                    p["id"].as_str().unwrap_or_default(),
                    p["interop"]
                        .as_array()
                        .map(|a| a
                            .iter()
                            .filter_map(Value::as_str)
                            .collect::<Vec<_>>()
                            .join(","))
                        .unwrap_or_default(),
                    p["description"].as_str().unwrap_or_default()
                );
            }
        }
        Cmd::Enable { plugin } => {
            c.call(
                "setPluginEnabled",
                json!({ "plugin": plugin, "enabled": true }),
            )
            .await?;
        }
        Cmd::Disable { plugin } => {
            c.call(
                "setPluginEnabled",
                json!({ "plugin": plugin, "enabled": false }),
            )
            .await?;
        }
        Cmd::Action {
            plugin,
            action: a,
            device,
            args,
        } => {
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
