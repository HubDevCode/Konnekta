//! Configurable actions and macros, shared by custom commands and the Deck.
//!
//! Actions are always defined by the *local* user. A remote device can only
//! trigger them by id; it can never send command text to execute.

use crate::runcommand::RunCommandPlugin;
use anyhow::{Result, bail};
use konnekta_core::CoreHandle;
use konnekta_platform::input::InputEvent;
use konnekta_platform::{MediaAction, Platform, parse_shortcut};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;

pub const MAX_DELAY_MS: u64 = 60_000;
pub const MAX_STEPS: usize = 100;
pub const MAX_DEPTH: u8 = 4;
pub const DEFAULT_SHELL_TIMEOUT: u64 = 60;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Action {
    /// Shell command (sh -c / cmd /C).
    Shell {
        command: String,
        #[serde(default, rename = "timeoutSecs", skip_serializing_if = "Option::is_none")]
        timeout_secs: Option<u64>,
    },
    /// Start an application.
    LaunchApp {
        app: String,
        #[serde(default)]
        args: Vec<String>,
    },
    /// Open a file, folder or URL.
    Open { target: String },
    /// Keyboard shortcut, e.g. `ctrl+shift+m`.
    Shortcut { keys: String },
    /// Type text.
    TypeText { text: String },
    /// Media control: Play, Pause, PlayPause, Stop, Next, Previous.
    Media { action: String },
    /// System volume: absolute (`set`) or relative (`delta`).
    Volume {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        set: Option<u8>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        delta: Option<i16>,
    },
    /// Wait between macro steps.
    Delay { ms: u64 },
    /// Run a saved custom command (by id).
    Command { id: String },
    /// Call any plugin action (extension point for the Deck).
    Plugin {
        plugin: String,
        action: String,
        #[serde(default)]
        args: Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        device: Option<String>,
    },
}

impl Action {
    /// Short human description (shown to KDE Connect clients, never secret).
    pub fn describe(&self) -> String {
        match self {
            Self::Shell { .. } => "Shell command".into(),
            Self::LaunchApp { app, .. } => format!("Launch {app}"),
            Self::Open { .. } => "Open".into(),
            Self::Shortcut { keys } => format!("Shortcut {keys}"),
            Self::TypeText { .. } => "Type text".into(),
            Self::Media { action } => format!("Media {action}"),
            Self::Volume { .. } => "Volume".into(),
            Self::Delay { ms } => format!("Wait {ms} ms"),
            Self::Command { .. } => "Command".into(),
            Self::Plugin { plugin, action, .. } => format!("{plugin}.{action}"),
        }
    }

    /// Validate user input before saving.
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::Shell { command, .. } if command.trim().is_empty() => bail!("empty shell command"),
            Self::LaunchApp { app, .. } if app.trim().is_empty() => bail!("empty application"),
            Self::Shortcut { keys } => parse_shortcut(keys).map(drop),
            Self::Media { action } if MediaAction::parse(action).is_none() => bail!("unknown media action {action}"),
            Self::Delay { ms } if *ms > MAX_DELAY_MS => bail!("delay too long (max {MAX_DELAY_MS} ms)"),
            _ => Ok(()),
        }
    }
}

pub fn validate_all(actions: &[Action]) -> Result<()> {
    if actions.len() > MAX_STEPS {
        bail!("too many steps (max {MAX_STEPS})");
    }
    actions.iter().try_for_each(Action::validate)
}

/// Executes actions with the platform backends.
#[derive(Clone)]
pub struct Executor {
    platform: Platform,
    commands: Arc<RunCommandPlugin>,
}

impl Executor {
    pub fn new(platform: Platform, commands: Arc<RunCommandPlugin>) -> Self {
        Self { platform, commands }
    }

    pub async fn run(&self, core: Option<&CoreHandle>, actions: &[Action]) -> Result<Vec<Value>> {
        self.run_depth(core, actions, 0).await
    }

    fn run_depth<'a>(
        &'a self,
        core: Option<&'a CoreHandle>,
        actions: &'a [Action],
        depth: u8,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<Value>>> + Send + 'a>> {
        Box::pin(async move {
            if depth > MAX_DEPTH {
                bail!("macro nesting too deep");
            }
            validate_all(actions)?;
            let mut results = Vec::new();
            for a in actions {
                results.push(self.run_one(core, a, depth).await?);
            }
            Ok(results)
        })
    }

    async fn run_one(&self, core: Option<&CoreHandle>, a: &Action, depth: u8) -> Result<Value> {
        let p = &self.platform;
        Ok(match a {
            Action::Shell { command, timeout_secs } => {
                let t = Duration::from_secs(timeout_secs.unwrap_or(DEFAULT_SHELL_TIMEOUT).min(3600));
                serde_json::to_value(p.launcher.run_shell(command, t).await?)?
            }
            Action::LaunchApp { app, args } => {
                p.launcher.launch_app(app, args).await?;
                Value::Null
            }
            Action::Open { target } => {
                p.launcher.open(target).await?;
                Value::Null
            }
            Action::Shortcut { keys } => {
                let (modifiers, key) = parse_shortcut(keys)?;
                p.input.send(InputEvent::Key { key, modifiers })?;
                Value::Null
            }
            Action::TypeText { text } => {
                p.input.send(InputEvent::Text { text: text.clone() })?;
                Value::Null
            }
            Action::Media { action } => {
                let m = MediaAction::parse(action).ok_or_else(|| anyhow::anyhow!("bad media action"))?;
                p.media.action(None, m).await?;
                Value::Null
            }
            Action::Volume { set, delta } => {
                let target = match (set, delta) {
                    (Some(v), _) => *v,
                    (None, Some(d)) => {
                        let cur = p.media.volume().await?.unwrap_or(50) as i16;
                        (cur + d).clamp(0, 100) as u8
                    }
                    _ => bail!("volume action needs set or delta"),
                };
                p.media.set_volume(target).await?;
                json!({ "volume": target })
            }
            Action::Delay { ms } => {
                tokio::time::sleep(Duration::from_millis((*ms).min(MAX_DELAY_MS))).await;
                Value::Null
            }
            Action::Command { id } => {
                let actions = self
                    .commands
                    .command(id)
                    .ok_or_else(|| anyhow::anyhow!("unknown command {id}"))?
                    .actions;
                Value::Array(self.run_depth(core, &actions, depth + 1).await?)
            }
            Action::Plugin { plugin, action, args, device } => {
                let core = core.ok_or_else(|| anyhow::anyhow!("plugin actions need the core"))?;
                core.plugin_action(device.as_deref(), plugin, action, args.clone()).await?
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use konnekta_platform::mock::MockPlatform;

    #[tokio::test]
    async fn runs_macro_in_order_with_delay() {
        let mock = MockPlatform::new();
        let platform = mock.platform();
        let cmds = RunCommandPlugin::new(platform.clone());
        let exec = Executor::new(platform, cmds);
        let start = std::time::Instant::now();
        exec.run(
            None,
            &[
                Action::Shortcut { keys: "ctrl+c".into() },
                Action::Delay { ms: 50 },
                Action::Media { action: "Next".into() },
                Action::Volume { set: None, delta: Some(10) },
                Action::Shell { command: "echo hi".into(), timeout_secs: None },
            ],
        )
        .await
        .unwrap();
        assert!(start.elapsed() >= Duration::from_millis(50));
        let calls = mock.calls();
        assert!(calls[0].starts_with("input:") && calls[0].contains("\"ctrl\":true"));
        assert_eq!(calls[1], "media:Next");
        assert_eq!(calls[2], "volume:10");
        assert_eq!(calls[3], "shell:echo hi");
    }

    #[test]
    fn validation() {
        assert!(Action::Shortcut { keys: "ctrl+nope".into() }.validate().is_err());
        assert!(Action::Delay { ms: MAX_DELAY_MS + 1 }.validate().is_err());
        assert!(Action::Media { action: "Explode".into() }.validate().is_err());
        assert!(validate_all(&vec![Action::Delay { ms: 1 }; MAX_STEPS + 1]).is_err());
        let a: Action = serde_json::from_str(r#"{"type":"shortcut","keys":"alt+tab"}"#).unwrap();
        assert!(a.validate().is_ok());
    }
}
