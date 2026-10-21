//! Setup assistant: which helper programs and kernel modules each media
//! feature needs on this computer, which are missing, and the commands that
//! install or enable them for the detected system. Nothing is run as root:
//! the user copies the commands.

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Status {
    Ok,
    /// Not installed.
    Missing,
    /// Installed but not active (kernel module not loaded).
    NotLoaded,
    /// Cannot work in this session (e.g. Wayland compositor without wlroots).
    Unsupported,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Requirement {
    /// Program or module name.
    pub id: &'static str,
    pub status: Status,
    /// What it is for, shown next to the name.
    pub purpose: &'static str,
    /// Commands that fix it, in order.
    pub commands: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Feature {
    /// `microphone`, `webcam`, `phoneScreen`, `desktop`, `systemAudio`,
    /// `phoneAudio`, `media`, `mixer`.
    pub id: &'static str,
    pub ok: bool,
    pub requirements: Vec<Requirement>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    /// `linux`, `windows` or `macos`.
    pub os: &'static str,
    /// Distribution family the commands are written for (Linux only).
    pub distro: Option<&'static str>,
    pub features: Vec<Feature>,
    /// Every fix of the report in one block, duplicates removed.
    pub all_commands: Vec<String>,
}

/// Inspect this computer.
pub fn check() -> Report {
    let env = Env::detect();
    let features = features(&env);
    let mut all_commands: Vec<String> = Vec::new();
    // One install line for every missing package, then the other steps.
    let pkgs: Vec<&str> = {
        let mut v: Vec<&str> = features
            .iter()
            .flat_map(|f| &f.requirements)
            .filter(|r| r.status == Status::Missing)
            .filter_map(|r| package(env.distro, r.id))
            .collect();
        v.sort();
        v.dedup();
        v
    };
    if !pkgs.is_empty()
        && let Some(cmd) = install(&env, &pkgs.join(" "))
    {
        all_commands.push(cmd);
    }
    for r in features.iter().flat_map(|f| &f.requirements) {
        if r.status == Status::Ok {
            continue;
        }
        let skip_install = usize::from(r.status == Status::Missing && package(env.distro, r.id).is_some());
        for c in r.commands.iter().skip(skip_install) {
            if !all_commands.contains(c) {
                all_commands.push(c.clone());
            }
        }
    }
    Report {
        os: env.os,
        distro: env.distro,
        features,
        all_commands,
    }
}

struct Env {
    os: &'static str,
    distro: Option<&'static str>,
    wayland: bool,
    x11: bool,
    desktop: String,
}

impl Env {
    fn detect() -> Self {
        let os = if cfg!(windows) {
            "windows"
        } else if cfg!(target_os = "macos") {
            "macos"
        } else {
            "linux"
        };
        Self {
            os,
            distro: (os == "linux")
                .then(|| distro(&std::fs::read_to_string("/etc/os-release").unwrap_or_default()))
                .flatten(),
            wayland: std::env::var_os("WAYLAND_DISPLAY").is_some(),
            x11: std::env::var_os("DISPLAY").is_some(),
            desktop: std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default(),
        }
    }
}

/// Family of the distribution from `/etc/os-release`.
fn distro(os_release: &str) -> Option<&'static str> {
    let field = |k: &str| {
        os_release
            .lines()
            .find_map(|l| l.strip_prefix(k)?.strip_prefix('='))
            .map(|v| v.trim_matches('"').to_lowercase())
            .unwrap_or_default()
    };
    let ids = format!("{} {}", field("ID"), field("ID_LIKE"));
    let has = |w: &str| ids.split_whitespace().any(|x| x == w);
    [
        ("arch", "arch"),
        ("debian", "debian"),
        ("ubuntu", "debian"),
        ("fedora", "fedora"),
        ("rhel", "fedora"),
        ("opensuse", "opensuse"),
        ("suse", "opensuse"),
    ]
    .into_iter()
    .find(|(w, _)| has(w))
    .map(|(_, d)| d)
}

/// Package providing a program or module, per distribution family.
fn package(distro: Option<&str>, id: &str) -> Option<&'static str> {
    let d = distro?;
    Some(match (id, d) {
        ("ffmpeg" | "ffplay", "fedora") => "ffmpeg-free",
        ("ffmpeg" | "ffplay", _) => "ffmpeg",
        ("pactl" | "pacat" | "parec", "arch") => "libpulse",
        ("pactl" | "pacat" | "parec", _) => "pulseaudio-utils",
        ("wf-recorder", _) => "wf-recorder",
        ("playerctl", _) => "playerctl",
        ("v4l2loopback", "arch") => "v4l2loopback-dkms linux-headers",
        ("v4l2loopback", "debian") => "v4l2loopback-dkms",
        ("v4l2loopback", "fedora") => "akmod-v4l2loopback",
        ("v4l2loopback", "opensuse") => "v4l2loopback-kmp-default",
        _ => return None,
    })
}

fn install(env: &Env, pkgs: &str) -> Option<String> {
    Some(match (env.os, env.distro) {
        (_, Some("arch")) => format!("sudo pacman -S --needed {pkgs}"),
        (_, Some("debian")) => format!("sudo apt install {pkgs}"),
        (_, Some("fedora")) => format!("sudo dnf install {pkgs}"),
        (_, Some("opensuse")) => format!("sudo zypper install {pkgs}"),
        _ => return None,
    })
}

const V4L2_OPTIONS: &str = "card_label=\"Konnekta Camera\" exclusive_caps=1";

fn program(env: &Env, id: &'static str, purpose: &'static str) -> Requirement {
    let ok = crate::pipeline::which(id);
    let commands = if ok {
        vec![]
    } else {
        match env.os {
            "windows" => vec![match id {
                "ffmpeg" | "ffplay" => "winget install Gyan.FFmpeg".into(),
                _ => format!("installa {id}"),
            }],
            "macos" => vec![match id {
                "ffmpeg" | "ffplay" => "brew install ffmpeg".into(),
                _ => format!("brew install {id}"),
            }],
            _ => package(env.distro, id)
                .and_then(|p| install(env, p))
                .into_iter()
                .collect(),
        }
    };
    Requirement {
        id,
        status: if ok { Status::Ok } else { Status::Missing },
        purpose,
        commands,
    }
}

#[cfg(target_os = "linux")]
fn v4l2loopback(env: &Env) -> Requirement {
    let loaded = crate::virtual_devices::find_v4l2loopback().is_some();
    let installed = loaded
        || std::process::Command::new("modinfo")
            .args(["-n", "v4l2loopback"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
    let mut commands = vec![];
    if !installed && let Some(i) = package(env.distro, "v4l2loopback").and_then(|p| install(env, p)) {
        commands.push(i);
    }
    if !loaded {
        commands.push(format!("sudo modprobe v4l2loopback {V4L2_OPTIONS}"));
        // Load it again at every boot.
        commands.push(
            "echo v4l2loopback | sudo tee /etc/modules-load.d/konnekta-v4l2loopback.conf".into(),
        );
        commands.push(format!(
            "echo 'options v4l2loopback {V4L2_OPTIONS}' | sudo tee /etc/modprobe.d/konnekta-v4l2loopback.conf"
        ));
    }
    Requirement {
        id: "v4l2loopback",
        status: match (loaded, installed) {
            (true, _) => Status::Ok,
            (false, true) => Status::NotLoaded,
            (false, false) => Status::Missing,
        },
        purpose: "webcam virtuale del kernel",
        commands,
    }
}

fn feature(id: &'static str, requirements: Vec<Requirement>) -> Feature {
    Feature {
        id,
        ok: requirements.iter().all(|r| r.status == Status::Ok),
        requirements,
    }
}

fn features(env: &Env) -> Vec<Feature> {
    let ffmpeg = || program(env, "ffmpeg", "codifica e decodifica video");
    let ffplay = || program(env, "ffplay", "finestra con lo schermo del telefono");
    if env.os != "linux" {
        let obs = Requirement {
            id: "obs",
            status: Status::Ok,
            purpose: "Virtual Camera di OBS: aggiungi una sorgente Media con l'indirizzo che Konnekta mostra",
            commands: vec![if env.os == "windows" {
                "winget install OBSProject.OBSStudio".into()
            } else {
                "brew install --cask obs".into()
            }],
        };
        return vec![
            feature("webcam", vec![ffmpeg(), obs]),
            feature("phoneScreen", vec![ffplay()]),
            feature("desktop", vec![ffmpeg()]),
        ];
    }
    let pulse = |id: &'static str, purpose| program(env, id, purpose);
    let desktop = if env.wayland {
        let wf = program(env, "wf-recorder", "cattura dello schermo su Wayland (wlroots)");
        let wlroots = !["GNOME", "KDE"].iter().any(|d| env.desktop.contains(d));
        if wf.status == Status::Ok || wlroots {
            vec![wf]
        } else if env.x11 {
            // GNOME / Plasma on Wayland: fall back to XWayland capture.
            vec![ffmpeg()]
        } else {
            vec![Requirement {
                id: "wf-recorder",
                status: Status::Unsupported,
                purpose: "questo desktop Wayland non è wlroots: accedi con una sessione X11 o usa Hyprland/Sway",
                commands: vec![],
            }]
        }
    } else {
        vec![ffmpeg()]
    };
    #[cfg(target_os = "linux")]
    let webcam = vec![v4l2loopback(env), ffmpeg()];
    #[cfg(not(target_os = "linux"))]
    let webcam = vec![ffmpeg()];
    vec![
        feature("microphone", vec![pulse("pactl", "crea il microfono virtuale")]),
        feature("webcam", webcam),
        feature("phoneScreen", vec![ffplay()]),
        feature("desktop", desktop),
        feature("systemAudio", vec![pulse("parec", "registra l'audio del computer")]),
        feature("phoneAudio", vec![pulse("pacat", "riproduce l'audio del telefono")]),
        feature("media", vec![program(env, "playerctl", "controlla i lettori (MPRIS)")]),
        feature("mixer", vec![pulse("pactl", "volumi di uscite, ingressi e app")]),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_distribution_family() {
        assert_eq!(distro("ID=arch\n"), Some("arch"));
        assert_eq!(distro("ID=endeavouros\nID_LIKE=arch\n"), Some("arch"));
        assert_eq!(distro("ID=ubuntu\nID_LIKE=debian\n"), Some("debian"));
        assert_eq!(distro("ID=\"linuxmint\"\nID_LIKE=\"ubuntu debian\"\n"), Some("debian"));
        assert_eq!(distro("ID=fedora\n"), Some("fedora"));
        assert_eq!(distro("ID=\"opensuse-tumbleweed\"\nID_LIKE=\"opensuse suse\"\n"), Some("opensuse"));
        assert_eq!(distro("ID=nixos\n"), None);
    }

    #[test]
    fn packages_per_family() {
        assert_eq!(package(Some("arch"), "pactl"), Some("libpulse"));
        assert_eq!(package(Some("debian"), "parec"), Some("pulseaudio-utils"));
        assert_eq!(package(Some("fedora"), "ffplay"), Some("ffmpeg-free"));
        assert_eq!(package(None, "ffmpeg"), None);
    }

    #[test]
    fn report_on_this_machine() {
        let r = check();
        assert!(!r.features.is_empty());
        for f in &r.features {
            assert_eq!(f.ok, f.requirements.iter().all(|x| x.status == Status::Ok));
        }
    }
}
