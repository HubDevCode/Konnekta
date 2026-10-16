use anyhow::Result;
use async_trait::async_trait;
#[cfg(feature = "desktop")]
use std::sync::Arc;

#[async_trait]
pub trait ClipboardBackend: Send + Sync {
    async fn get_text(&self) -> Result<Option<String>>;
    async fn set_text(&self, text: &str) -> Result<()>;
    /// Put a PNG image on the clipboard (screenshots).
    async fn set_png(&self, png: &[u8]) -> Result<()>;
}

#[cfg(feature = "desktop")]
pub fn native() -> Arc<dyn ClipboardBackend> {
    #[cfg(target_os = "linux")]
    if std::env::var_os("WAYLAND_DISPLAY").is_some() && crate::cmd::exists("wl-copy") {
        // Works on every compositor, including GNOME (no data-control).
        return Arc::new(WlClipboard);
    }
    Arc::new(arboard_backend::ArboardClipboard::spawn())
}

#[cfg(all(feature = "desktop", target_os = "linux"))]
pub struct WlClipboard;

#[cfg(all(feature = "desktop", target_os = "linux"))]
#[async_trait]
impl ClipboardBackend for WlClipboard {
    async fn get_text(&self) -> Result<Option<String>> {
        let types = crate::cmd::output("wl-paste", &["--list-types"]).await.unwrap_or_default();
        if !types.lines().any(|t| t.starts_with("text/") || t == "UTF8_STRING" || t == "STRING") {
            return Ok(None);
        }
        Ok(crate::cmd::output("wl-paste", &["--no-newline", "--type", "text"]).await.ok())
    }

    async fn set_text(&self, text: &str) -> Result<()> {
        crate::cmd::with_stdin("wl-copy", &["--type", "text/plain;charset=utf-8"], text.as_bytes()).await
    }

    async fn set_png(&self, png: &[u8]) -> Result<()> {
        crate::cmd::with_stdin("wl-copy", &["--type", "image/png"], png).await
    }
}

#[cfg(feature = "desktop")]
mod arboard_backend {
    use super::*;
    use anyhow::Context;
    use std::sync::mpsc;
    use tokio::sync::oneshot;

    enum Cmd {
        Get(oneshot::Sender<Result<Option<String>>>),
        SetText(String, oneshot::Sender<Result<()>>),
        SetImage(Vec<u8>, usize, usize, oneshot::Sender<Result<()>>),
    }

    /// arboard owns the selection on X11 only while the `Clipboard` lives, so
    /// a single instance is kept on a dedicated thread.
    pub struct ArboardClipboard {
        tx: mpsc::Sender<Cmd>,
    }

    impl ArboardClipboard {
        pub fn spawn() -> Self {
            let (tx, rx) = mpsc::channel::<Cmd>();
            std::thread::Builder::new()
                .name("konnekta-clipboard".into())
                .spawn(move || {
                    let mut cb = None;
                    while let Ok(cmd) = rx.recv() {
                        if cb.is_none() {
                            cb = arboard::Clipboard::new().ok();
                        }
                        let Some(c) = cb.as_mut() else {
                            let err = || anyhow::anyhow!("clipboard unavailable");
                            match cmd {
                                Cmd::Get(r) => drop(r.send(Err(err()))),
                                Cmd::SetText(_, r) | Cmd::SetImage(_, _, _, r) => drop(r.send(Err(err()))),
                            }
                            continue;
                        };
                        match cmd {
                            Cmd::Get(r) => {
                                let _ = r.send(Ok(c.get_text().ok()));
                            }
                            Cmd::SetText(t, r) => {
                                let _ = r.send(c.set_text(t).map_err(Into::into));
                            }
                            Cmd::SetImage(bytes, w, h, r) => {
                                let img = arboard::ImageData { width: w, height: h, bytes: bytes.into() };
                                let _ = r.send(c.set_image(img).map_err(Into::into));
                            }
                        }
                    }
                })
                .expect("spawn clipboard thread");
            Self { tx }
        }

        async fn call<T>(&self, f: impl FnOnce(oneshot::Sender<Result<T>>) -> Cmd) -> Result<T> {
            let (tx, rx) = oneshot::channel();
            self.tx.send(f(tx)).map_err(|_| anyhow::anyhow!("clipboard thread stopped"))?;
            rx.await.context("clipboard thread stopped")?
        }
    }

    #[async_trait]
    impl ClipboardBackend for ArboardClipboard {
        async fn get_text(&self) -> Result<Option<String>> {
            self.call(Cmd::Get).await
        }

        async fn set_text(&self, text: &str) -> Result<()> {
            let t = text.to_owned();
            self.call(|r| Cmd::SetText(t, r)).await
        }

        async fn set_png(&self, png: &[u8]) -> Result<()> {
            let img = image::load_from_memory_with_format(png, image::ImageFormat::Png)?.to_rgba8();
            let (w, h) = (img.width() as usize, img.height() as usize);
            let raw = img.into_raw();
            self.call(|r| Cmd::SetImage(raw, w, h, r)).await
        }
    }
}
