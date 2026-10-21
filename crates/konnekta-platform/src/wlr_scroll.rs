//! Smooth scrolling on wlroots compositors (Hyprland, Sway…).
//!
//! enigo can only send whole wheel notches, which feels jerky from a phone
//! touchpad. A `zwlr_virtual_pointer_v1` lets us send scroll distances in
//! pixels with the "finger" source, exactly what a laptop touchpad sends,
//! so applications scroll smoothly (and may add their own inertia).

use anyhow::{Context, Result};
use std::time::Instant;
use wayland_client::protocol::{wl_pointer, wl_registry};
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle};
use wayland_protocols_wlr::virtual_pointer::v1::client::{
    zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1,
    zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1,
};

#[derive(Default)]
struct State {
    manager: Option<ZwlrVirtualPointerManagerV1>,
}

impl Dispatch<wl_registry::WlRegistry, ()> for State {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
            && interface == "zwlr_virtual_pointer_manager_v1"
        {
            state.manager = Some(registry.bind(name, version.min(2), qh, ()));
        }
    }
}

impl Dispatch<ZwlrVirtualPointerManagerV1, ()> for State {
    fn event(
        _: &mut Self,
        _: &ZwlrVirtualPointerManagerV1,
        _: <ZwlrVirtualPointerManagerV1 as wayland_client::Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwlrVirtualPointerV1, ()> for State {
    fn event(
        _: &mut Self,
        _: &ZwlrVirtualPointerV1,
        _: <ZwlrVirtualPointerV1 as wayland_client::Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

pub struct WlrScroll {
    conn: Connection,
    _queue: EventQueue<State>,
    pointer: ZwlrVirtualPointerV1,
    start: Instant,
}

impl WlrScroll {
    /// `None` when not on Wayland or the compositor lacks the protocol.
    pub fn connect() -> Result<Self> {
        let conn = Connection::connect_to_env().context("no Wayland display")?;
        let mut queue = conn.new_event_queue();
        let qh = queue.handle();
        conn.display().get_registry(&qh, ());
        let mut state = State::default();
        queue.roundtrip(&mut state)?;
        let manager = state
            .manager
            .clone()
            .context("the compositor has no virtual pointer protocol")?;
        let pointer = manager.create_virtual_pointer(None, &qh, ());
        queue.roundtrip(&mut state)?;
        Ok(Self {
            conn,
            _queue: queue,
            pointer,
            start: Instant::now(),
        })
    }

    /// Scroll by pixels: positive `dy` scrolls down, positive `dx` right.
    pub fn scroll(&mut self, dx: f64, dy: f64) -> Result<()> {
        let t = self.start.elapsed().as_millis() as u32;
        self.pointer.axis_source(wl_pointer::AxisSource::Finger);
        if dy != 0.0 {
            self.pointer.axis(t, wl_pointer::Axis::VerticalScroll, dy);
        }
        if dx != 0.0 {
            self.pointer.axis(t, wl_pointer::Axis::HorizontalScroll, dx);
        }
        self.pointer.frame();
        self.conn.flush()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    /// Needs a wlroots session: `cargo test -p konnekta-platform -- --ignored wlr`.
    #[test]
    #[ignore]
    fn wlr_connects() {
        let mut w = super::WlrScroll::connect().unwrap();
        w.scroll(0.0, 0.0).unwrap();
    }
}
