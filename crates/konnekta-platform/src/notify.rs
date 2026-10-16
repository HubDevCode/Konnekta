use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopNotification {
    /// Stable id so updates/cancellations replace the same notification.
    pub id: String,
    pub app_name: String,
    pub title: String,
    pub body: String,
    /// (action id, label) buttons; result delivered to `on_action`.
    #[serde(default)]
    pub actions: Vec<(String, String)>,
    #[serde(default)]
    pub urgent: bool,
}

pub type ActionCallback = Box<dyn FnOnce(String) + Send + 'static>;

pub trait NotifierBackend: Send + Sync {
    fn show(&self, n: DesktopNotification, on_action: Option<ActionCallback>) -> Result<()>;
    fn close(&self, _id: &str) {}
}

#[cfg(feature = "desktop")]
pub struct NativeNotifier;

#[cfg(feature = "desktop")]
impl NotifierBackend for NativeNotifier {
    fn show(&self, n: DesktopNotification, on_action: Option<ActionCallback>) -> Result<()> {
        // notify-rust blocks while waiting for actions: run on a blocking thread.
        std::thread::spawn(move || {
            let mut notif = notify_rust::Notification::new();
            notif
                .appname("Konnekta")
                .summary(&if n.app_name.is_empty() {
                    n.title.clone()
                } else {
                    format!("{} — {}", n.app_name, n.title)
                })
                .body(&n.body);
            #[cfg(all(unix, not(target_os = "macos")))]
            {
                for (id, label) in &n.actions {
                    notif.action(id, label);
                }
                if n.urgent {
                    notif.urgency(notify_rust::Urgency::Critical);
                }
            }
            match notif.show() {
                #[cfg(all(unix, not(target_os = "macos")))]
                Ok(handle) => {
                    if let Some(cb) = on_action {
                        handle.wait_for_action(|a| {
                            if a != "__closed" {
                                cb(a.to_owned());
                            }
                        });
                    }
                }
                #[cfg(not(all(unix, not(target_os = "macos"))))]
                Ok(_) => drop(on_action),
                Err(e) => tracing::warn!("cannot show notification: {e}"),
            }
        });
        Ok(())
    }
}
