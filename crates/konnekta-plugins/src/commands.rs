//! Custom commands (`konnekta.commands`).
//!
//! Commands are created on this computer by the local user: a name, icon,
//! description, group and a list of [`Action`]s (shell, launch app, open,
//! shortcut, media, macro steps...). Paired devices with the *Commands*
//! permission receive the list (names only, never the command text for
//! Konnekta peers) and can trigger a command **by key**. Nothing sent over the
//! network is ever interpreted as a command line.

use crate::actions::{Action, Executor, validate_all};
use crate::store::JsonStore;
use anyhow::{Result, bail};
use async_trait::async_trait;
use konnekta_core::proto::bodies::{RunCommandBody, RunCommandEntry, RunCommandRequestBody};
use konnekta_core::proto::{Packet, types};
use konnekta_core::{CoreHandle, DeviceHandle, Permission, Plugin, PluginInfo};
use konnekta_platform::{DesktopNotification, Platform};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, OnceLock, Weak};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandEntry {
    #[serde(default)]
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub icon: String,
    #[serde(default)]
    pub group: String,
    pub actions: Vec<Action>,
    /// Show a desktop notification with the result.
    #[serde(default)]
    pub notify: bool,
}

impl CommandEntry {
    fn validate(&self) -> Result<()> {
        if self.name.trim().is_empty() || self.name.chars().count() > 64 {
            bail!("invalid command name");
        }
        if self.actions.is_empty() {
            bail!("a command needs at least one action");
        }
        validate_all(&self.actions)
    }

    /// Text shown by KDE Connect clients in the `command` field.
    fn summary(&self) -> String {
        if !self.description.is_empty() {
            return self.description.clone();
        }
        self.actions
            .iter()
            .map(Action::describe)
            .collect::<Vec<_>>()
            .join(", ")
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CommandsConfig {
    pub commands: Vec<CommandEntry>,
}

pub struct CommandsPlugin {
    platform: Platform,
    me: Weak<CommandsPlugin>,
    core: OnceLock<CoreHandle>,
    pub config: JsonStore<CommandsConfig>,
    remote: Mutex<HashMap<String, BTreeMap<String, RunCommandEntry>>>,
}

impl CommandsPlugin {
    pub fn new(platform: Platform) -> Arc<Self> {
        Arc::new_cyclic(|me| Self {
            platform,
            me: me.clone(),
            core: OnceLock::new(),
            config: JsonStore::default(),
            remote: Mutex::default(),
        })
    }

    pub fn command(&self, id: &str) -> Option<CommandEntry> {
        self.config
            .with(|c| c.commands.iter().find(|e| e.id == id).cloned())
    }

    fn executor(&self) -> Result<Executor> {
        let me = self
            .me
            .upgrade()
            .ok_or_else(|| anyhow::anyhow!("plugin dropped"))?;
        Ok(Executor::new(self.platform.clone(), me))
    }

    pub async fn run(&self, id: &str) -> Result<Vec<Value>> {
        let cmd = self
            .command(id)
            .ok_or_else(|| anyhow::anyhow!("unknown command {id}"))?;
        let out = self.executor()?.run(self.core.get(), &cmd.actions).await;
        if cmd.notify {
            let body = match &out {
                Ok(_) => "Completed".to_string(),
                Err(e) => format!("Failed: {e}"),
            };
            let _ = self.platform.notifier.show(
                DesktopNotification {
                    id: format!("cmd-{id}"),
                    title: cmd.name.clone(),
                    body,
                    ..Default::default()
                },
                None,
            );
        }
        out
    }

    fn list_packet(&self) -> Packet {
        let map: BTreeMap<String, RunCommandEntry> = self.config.with(|c| {
            c.commands
                .iter()
                .map(|e| {
                    (
                        e.id.clone(),
                        RunCommandEntry {
                            name: e.name.clone(),
                            command: e.summary(),
                        },
                    )
                })
                .collect()
        });
        Packet::new(types::COMMANDS, RunCommandBody::from_map(&map))
    }

    async fn push_list(&self) {
        let Some(core) = self.core.get() else { return };
        for d in core.devices() {
            if d.has_permission(Permission::Commands) {
                let _ = d.send_if_supported(self.list_packet()).await;
            }
        }
    }

    fn save(&self, f: impl FnOnce(&mut Vec<CommandEntry>) -> Result<()>) -> Result<()> {
        self.config.update(|c| {
            let mut copy = c.commands.clone();
            f(&mut copy)?;
            let mut seen = std::collections::HashSet::new();
            for e in &copy {
                e.validate()?;
                if !seen.insert(e.id.clone()) {
                    bail!("duplicate command id {}", e.id);
                }
            }
            c.commands = copy;
            Ok(())
        })
    }
}

#[async_trait]
impl Plugin for CommandsPlugin {
    fn info(&self) -> PluginInfo {
        PluginInfo {
            id: "commands",
            name: "Custom commands",
            description: "Run commands configured on this computer from the phone",
            permission: Permission::Commands,
            incoming: &[types::COMMANDS_REQUEST, types::COMMANDS],
            outgoing: &[types::COMMANDS, types::COMMANDS_REQUEST],
        }
    }

    async fn start(self: Arc<Self>, core: CoreHandle) {
        self.config.attach(&core, "commands", "commands.json");
        let _ = self.core.set(core);
    }

    async fn on_connected(&self, dev: &DeviceHandle) {
        let _ = dev.send_if_supported(self.list_packet()).await;
    }

    async fn on_disconnected(&self, id: &str) {
        self.remote.lock().unwrap().remove(id);
    }

    async fn handle_packet(&self, dev: &DeviceHandle, p: Packet) -> Result<()> {
        match p.packet_type.as_str() {
            types::COMMANDS_REQUEST => {
                let req: RunCommandRequestBody = p.body_as()?;
                if req.request_command_list == Some(true) {
                    dev.send(self.list_packet()).await?;
                }
                if let Some(key) = req.key {
                    tracing::info!(device = %dev.name(), command = %key, "remote command request");
                    let result = self.run(&key).await;
                    dev.emit(
                        "commands",
                        "executed",
                        json!({ "id": key, "ok": result.is_ok(), "error": result.as_ref().err().map(|e| e.to_string()) }),
                    );
                    result?;
                }
                if p.get_bool("setup") {
                    dev.emit("commands", "setupRequested", Value::Null);
                }
            }
            types::COMMANDS => {
                let body: RunCommandBody = p.body_as()?;
                let entries = body.entries();
                dev.emit(
                    "commands",
                    "remoteCommands",
                    serde_json::to_value(&entries)?,
                );
                self.remote
                    .lock()
                    .unwrap()
                    .insert(dev.id().to_owned(), entries);
            }
            _ => {}
        }
        Ok(())
    }

    async fn handle_action(
        &self,
        dev: Option<&DeviceHandle>,
        action: &str,
        args: Value,
    ) -> Result<Value> {
        match action {
            "list" => Ok(serde_json::to_value(self.config.get().commands)?),
            "add" | "update" => {
                let mut e: CommandEntry = crate::args(args)?;
                if e.id.is_empty() {
                    e.id = uuid::Uuid::new_v4().simple().to_string();
                }
                let id = e.id.clone();
                let is_add = action == "add";
                self.save(|v| {
                    match v.iter_mut().find(|x| x.id == e.id) {
                        Some(x) if !is_add => *x = e,
                        Some(_) => bail!("command {} already exists", e.id),
                        None if is_add => v.push(e),
                        None => bail!("unknown command {}", e.id),
                    }
                    Ok(())
                })?;
                self.push_list().await;
                Ok(json!({ "id": id }))
            }
            "delete" => {
                let id = args
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                self.save(|v| {
                    v.retain(|x| x.id != id);
                    Ok(())
                })?;
                self.push_list().await;
                Ok(Value::Null)
            }
            "reorder" => {
                let ids: Vec<String> = crate::args(args.get("ids").cloned().unwrap_or_default())?;
                self.save(|v| {
                    v.sort_by_key(|e| ids.iter().position(|i| *i == e.id).unwrap_or(usize::MAX));
                    Ok(())
                })?;
                self.push_list().await;
                Ok(Value::Null)
            }
            "run" => {
                let id = args.get("id").and_then(Value::as_str).unwrap_or_default();
                Ok(Value::Array(self.run(id).await?))
            }
            "remoteList" => {
                let d = crate::need_dev(dev)?;
                d.send(Packet::new(
                    types::COMMANDS_REQUEST,
                    RunCommandRequestBody {
                        key: None,
                        request_command_list: Some(true),
                    },
                ))
                .await?;
                Ok(serde_json::to_value(
                    self.remote.lock().unwrap().get(d.id()),
                )?)
            }
            "runRemote" => {
                let key = args
                    .get("key")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                crate::need_dev(dev)?
                    .send(Packet::new(
                        types::COMMANDS_REQUEST,
                        RunCommandRequestBody {
                            key: Some(key),
                            request_command_list: None,
                        },
                    ))
                    .await?;
                Ok(Value::Null)
            }
            _ => bail!("unknown action {action}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use konnekta_platform::mock::MockPlatform;

    #[tokio::test]
    async fn crud_and_run() {
        let mock = MockPlatform::new();
        let p = CommandsPlugin::new(mock.platform());
        let r = p
            .handle_action(None, "add", json!({"name": "Lock", "actions": [{"type": "shell", "command": "loginctl lock-session"}]}))
            .await
            .unwrap();
        let id = r["id"].as_str().unwrap().to_owned();
        assert!(
            p.handle_action(None, "add", json!({"name": "", "actions": []}))
                .await
                .is_err()
        );
        p.run(&id).await.unwrap();
        assert_eq!(mock.calls(), vec!["shell:loginctl lock-session"]);
        // KDE list exposes names and descriptions only.
        let list = p.list_packet();
        let body: RunCommandBody = list.body_as().unwrap();
        assert_eq!(body.entries()[&id].name, "Lock");
        assert!(!body.command_list.contains("loginctl"));
        p.handle_action(None, "delete", json!({"id": id}))
            .await
            .unwrap();
        assert!(p.run(&id).await.is_err());
    }
}
