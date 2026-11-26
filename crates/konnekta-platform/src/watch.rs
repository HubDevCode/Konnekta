//! Helper programs that stay running and print a line at every change
//! (`playerctl --follow`, `pactl subscribe`, `wl-paste --watch`,
//! `gsettings monitor`): a value is read again only when something changed,
//! instead of starting a program every second to ask.
//!
//! A watcher starts the first time it is asked for, and stops by itself
//! once nobody asked for a while (no process kept for a feature not in use).
//! A program that is missing or ends at once (e.g. `wl-paste --watch` on a
//! compositor without data-control) is tried again later, less and less
//! often; meanwhile [`LineWatcher::generation`] answers `None` and the
//! caller polls as before.

use std::process::Stdio;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::watch;

/// The watcher stops when nobody asked for this long.
const IDLE: Duration = Duration::from_secs(120);
/// A program that ran less than this ended "at once" (unsupported here).
const QUICK_EXIT: Duration = Duration::from_secs(5);
const RETRY_MIN: Duration = Duration::from_secs(30);
const RETRY_MAX: Duration = Duration::from_secs(30 * 60);

pub struct LineWatcher {
    program: &'static str,
    args: &'static [&'static str],
    /// Which lines are a change worth reading again.
    filter: fn(&str) -> bool,
    generation: watch::Sender<u64>,
    state: Mutex<State>,
}

struct State {
    running: bool,
    last_used: Instant,
    retry_at: Option<Instant>,
    quick_exits: u32,
}

impl LineWatcher {
    /// Meant for a `static` (`LazyLock`): the follow task borrows it.
    pub fn new(
        program: &'static str,
        args: &'static [&'static str],
        filter: fn(&str) -> bool,
    ) -> Self {
        Self {
            program,
            args,
            filter,
            generation: watch::Sender::new(0),
            state: Mutex::new(State {
                running: false,
                last_used: Instant::now(),
                retry_at: None,
                quick_exits: 0,
            }),
        }
    }

    /// Changes seen so far, starting the watcher if needed. `None` when the
    /// watcher cannot run here (yet): poll instead. A value cached together
    /// with a generation stays valid while the generation is the same.
    pub fn generation(&'static self) -> Option<u64> {
        let now = Instant::now();
        {
            let mut s = self.state.lock().unwrap();
            s.last_used = now;
            if s.running {
                return Some(*self.generation.borrow());
            }
            if s.retry_at.is_some_and(|t| now < t) {
                return None;
            }
            tokio::runtime::Handle::try_current().ok()?;
            let child = crate::host::command_async(self.program)
                .args(self.args)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .kill_on_drop(true)
                .spawn();
            let mut child = match child {
                Ok(c) => c,
                Err(_) => {
                    s.retry_at = Some(now + RETRY_MAX);
                    return None;
                }
            };
            let Some(out) = child.stdout.take() else {
                s.retry_at = Some(now + RETRY_MIN);
                return None;
            };
            s.running = true;
            // Whatever was cached before the watcher ran is not trusted.
            self.bump();
            tokio::spawn(self.follow(child, out, now));
        }
        Some(*self.generation.borrow())
    }

    /// Wait for the next change (only meaningful while
    /// [`Self::generation`] answers `Some`).
    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.generation.subscribe()
    }

    fn bump(&self) {
        self.generation.send_modify(|g| *g = g.wrapping_add(1));
    }

    async fn follow(
        &'static self,
        child: tokio::process::Child,
        out: tokio::process::ChildStdout,
        started: Instant,
    ) {
        let _child = child; // killed when this ends
        let mut lines = BufReader::new(out).lines();
        let mut idle_check = tokio::time::interval(IDLE / 4);
        idle_check.tick().await;
        let mut stopped_idle = false;
        loop {
            tokio::select! {
                line = lines.next_line() => match line {
                    Ok(Some(l)) => {
                        if (self.filter)(&l) {
                            self.bump();
                        }
                    }
                    _ => break,
                },
                _ = idle_check.tick() => {
                    if self.state.lock().unwrap().last_used.elapsed() > IDLE {
                        stopped_idle = true;
                        break;
                    }
                }
            }
        }
        let mut s = self.state.lock().unwrap();
        s.running = false;
        if stopped_idle {
            s.quick_exits = 0;
            s.retry_at = None;
        } else if started.elapsed() < QUICK_EXIT {
            s.quick_exits = s.quick_exits.saturating_add(1);
            let wait = RETRY_MIN
                .saturating_mul(1 << s.quick_exits.min(6))
                .min(RETRY_MAX);
            s.retry_at = Some(Instant::now() + wait);
            tracing::debug!(
                program = self.program,
                "watcher ended at once, retry in {wait:?}"
            );
        } else {
            // Ended after running fine (player daemon restart...): soon again.
            s.quick_exits = 0;
            s.retry_at = Some(Instant::now() + Duration::from_secs(2));
        }
        drop(s);
        // Values cached while it ran may have changed unseen.
        self.bump();
    }
}

/// PulseAudio / PipeWire events about outputs, inputs, streams and the
/// defaults (`client` events are left out: each `pactl` call makes one).
#[cfg(all(feature = "desktop", target_os = "linux"))]
pub(crate) static PULSE: std::sync::LazyLock<LineWatcher> = std::sync::LazyLock::new(|| {
    LineWatcher::new("pactl", &["subscribe"], |l| {
        ["on sink", "on source", "on server", "on card"]
            .iter()
            .any(|k| l.contains(k))
    })
});

/// MPRIS players: a line whenever a player starts, stops, changes track or
/// playback state.
#[cfg(all(feature = "desktop", target_os = "linux"))]
pub(crate) static PLAYERS: std::sync::LazyLock<LineWatcher> = std::sync::LazyLock::new(|| {
    LineWatcher::new(
        "playerctl",
        &[
            "-a",
            "--follow",
            "metadata",
            "--format",
            "{{playerName}}\t{{status}}\t{{title}}\t{{artist}}\t{{album}}\t{{mpris:artUrl}}",
        ],
        |_| true,
    )
});

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::sync::LazyLock;

    static TICKS: LazyLock<LineWatcher> = LazyLock::new(|| {
        LineWatcher::new(
            "sh",
            &[
                "-c",
                "echo skip; sleep 0.2; echo a; sleep 0.2; echo b; sleep 30",
            ],
            |l| l != "skip",
        )
    });
    static MISSING: LazyLock<LineWatcher> =
        LazyLock::new(|| LineWatcher::new("konnekta-no-such-program", &[], |_| true));

    #[tokio::test]
    async fn counts_the_lines_that_pass_the_filter() {
        let g0 = TICKS.generation().expect("sh runs");
        let mut rx = TICKS.subscribe();
        tokio::time::timeout(Duration::from_secs(5), async {
            while *rx.borrow_and_update() < g0 + 2 {
                rx.changed().await.unwrap();
            }
        })
        .await
        .expect("two changes");
        // Still running: the same generation until the next line.
        let g = TICKS.generation().unwrap();
        assert_eq!(g, g0 + 2);
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(TICKS.generation(), Some(g));
    }

    #[tokio::test]
    async fn a_missing_program_means_polling() {
        assert_eq!(MISSING.generation(), None);
        assert_eq!(MISSING.generation(), None);
    }
}
