use winit::window::Window;

pub fn set_blur(window: &Window, enabled: bool, w: u32, h: u32, radius: u32) {
    window.set_blur(enabled);
    #[cfg(target_os = "linux")]
    x11::set_blur(window, enabled, w, h, radius);
    #[cfg(not(target_os = "linux"))]
    let _ = (w, h, radius);
}

pub fn set_dropdown_hints(window: &Window) {
    #[cfg(target_os = "linux")]
    x11::set_dropdown_hints(window);
    #[cfg(not(target_os = "linux"))]
    let _ = window;
}

#[cfg(target_os = "linux")]
mod x11 {
    use std::sync::OnceLock;

    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{AtomEnum, ConnectionExt, PropMode};
    use x11rb::rust_connection::RustConnection;
    use x11rb::wrapper::ConnectionExt as _;

    struct X {
        conn: RustConnection,
        atom: u32,
    }

    fn x() -> Option<&'static X> {
        static X: OnceLock<Option<X>> = OnceLock::new();
        X.get_or_init(|| {
            let (conn, _) = x11rb::connect(None).ok()?;
            let atom = conn.intern_atom(false, b"_KDE_NET_WM_BLUR_BEHIND_REGION").ok()?.reply().ok()?.atom;
            Some(X { conn, atom })
        })
        .as_ref()
    }

    fn window_id(window: &winit::window::Window) -> Option<u32> {
        match window.window_handle().map(|h| h.as_raw()) {
            Ok(RawWindowHandle::Xlib(h)) => Some(h.window as u32),
            Ok(RawWindowHandle::Xcb(h)) => Some(h.window.get()),
            _ => None,
        }
    }

    pub fn set_dropdown_hints(window: &winit::window::Window) {
        let (Some(id), Some(x)) = (window_id(window), x()) else { return };
        let atom = |name: &[u8]| x.conn.intern_atom(false, name).ok()?.reply().ok().map(|r| r.atom);
        let names: [&[u8]; 3] = [b"_NET_WM_STATE_SKIP_TASKBAR", b"_NET_WM_STATE_SKIP_PAGER", b"_NET_WM_STATE_ABOVE"];
        let states: Vec<u32> = names.iter().filter_map(|n| atom(n)).collect();
        let Some(prop) = atom(b"_NET_WM_STATE") else { return };

        if x.conn.change_property32(PropMode::REPLACE, id, prop, AtomEnum::ATOM, &states).is_ok() {
            let root = x.conn.setup().roots.first().map(|r| r.root);
            if let Some(root) = root {
                for s in &states {
                    let ev = x11rb::protocol::xproto::ClientMessageEvent::new(32, id, prop, [1, *s, 0, 1, 0]);
                    let mask = x11rb::protocol::xproto::EventMask::SUBSTRUCTURE_REDIRECT | x11rb::protocol::xproto::EventMask::SUBSTRUCTURE_NOTIFY;
                    let _ = x.conn.send_event(false, root, mask, ev);
                }
            }
            let _ = x.conn.flush();
        }
    }

    pub fn set_blur(window: &winit::window::Window, enabled: bool, w: u32, h: u32, r: u32) {
        let Some(id) = window_id(window) else { return };
        let Some(x) = x() else { return };
        let result = if enabled {
            let rects: Vec<u32> = if r == 0 || w <= 2 * r || h <= 2 * r {
                vec![0, 0, w, h]
            } else {
                vec![0, r, w, h - 2 * r, r, 0, w - 2 * r, r, r, h - r, w - 2 * r, r]
            };
            x.conn.change_property32(PropMode::REPLACE, id, x.atom, AtomEnum::CARDINAL, &rects).map(drop)
        } else {
            x.conn.delete_property(id, x.atom).map(drop)
        };
        if result.is_ok() {
            let _ = x.conn.flush();
        }
    }
}
