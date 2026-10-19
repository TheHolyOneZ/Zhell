use winit::window::Window;

pub enum Clipboard {
    #[cfg(all(unix, not(target_os = "macos")))]
    Wayland(smithay_clipboard::Clipboard),
    Arboard(arboard::Clipboard),
}

impl Clipboard {
    pub fn new(window: &Window) -> Option<Self> {
        #[cfg(all(unix, not(target_os = "macos")))]
        if let Some(c) = wayland(window) {
            return Some(Self::Wayland(c));
        }
        let _ = window;
        match arboard::Clipboard::new() {
            Ok(c) => Some(Self::Arboard(c)),
            Err(e) => {
                log::warn!("clipboard unavailable: {e}");
                None
            }
        }
    }

    pub fn get(&mut self) -> Option<String> {
        let res = match self {
            #[cfg(all(unix, not(target_os = "macos")))]
            Self::Wayland(c) => c.load().map_err(|e| e.to_string()),
            Self::Arboard(c) => c.get_text().map_err(|e| e.to_string()),
        };
        res.inspect_err(|e| log::warn!("clipboard: {e}")).ok()
    }

    pub fn set(&mut self, text: String) {
        match self {
            #[cfg(all(unix, not(target_os = "macos")))]
            Self::Wayland(c) => c.store(text),
            Self::Arboard(c) => {
                if let Err(e) = c.set_text(text) {
                    log::warn!("clipboard: {e}");
                }
            }
        }
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    pub fn get_primary(&mut self) -> Option<String> {
        use arboard::{GetExtLinux, LinuxClipboardKind};
        match self {
            Self::Wayland(c) => c.load_primary().ok(),
            Self::Arboard(c) => c.get().clipboard(LinuxClipboardKind::Primary).text().ok(),
        }
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    pub fn set_primary(&mut self, text: String) {
        use arboard::{LinuxClipboardKind, SetExtLinux};
        match self {
            Self::Wayland(c) => c.store_primary(text),
            Self::Arboard(c) => {
                if let Err(e) = c.set().clipboard(LinuxClipboardKind::Primary).text(text) {
                    log::warn!("clipboard: {e}");
                }
            }
        }
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
#[allow(unsafe_code)]
fn wayland(window: &Window) -> Option<smithay_clipboard::Clipboard> {
    use winit::raw_window_handle::{HasDisplayHandle, RawDisplayHandle};
    let RawDisplayHandle::Wayland(h) = window.display_handle().ok()?.as_raw() else { return None };
    Some(unsafe { smithay_clipboard::Clipboard::new(h.display.as_ptr()) })
}
