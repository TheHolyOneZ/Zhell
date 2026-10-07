use winit::keyboard::{Key, ModifiersState, NamedKey};
use zhell_core::layout::Rect;
use zhell_proto::{FindState, PaneId};
use zhell_render::Renderer;

use crate::draw::label;
use crate::theme::{RADIUS, RADIUS_SMALL, Rgba, Theme};

pub struct FindBar {
    pub pane: PaneId,
    pub query: String,
    pub regex: bool,
}

pub enum FindEffect {
    None,

    Search,
    Next { older: bool },
    Close,
}

impl FindBar {
    pub fn new(pane: PaneId) -> Self {
        Self { pane, query: String::new(), regex: false }
    }

    pub fn key(&mut self, key: &Key, text: Option<&str>, m: ModifiersState) -> FindEffect {
        match key {
            Key::Named(NamedKey::Escape) => FindEffect::Close,
            Key::Named(NamedKey::Enter) => FindEffect::Next { older: !m.shift_key() },
            Key::Named(NamedKey::ArrowUp) => FindEffect::Next { older: true },
            Key::Named(NamedKey::ArrowDown) => FindEffect::Next { older: false },
            Key::Named(NamedKey::Backspace) => {
                self.query.pop();
                FindEffect::Search
            }
            Key::Character(c) if m.alt_key() && c.eq_ignore_ascii_case("r") => {
                self.regex = !self.regex;
                FindEffect::Search
            }
            Key::Character(c) if m.control_key() && c.eq_ignore_ascii_case("u") => {
                self.query.clear();
                FindEffect::Search
            }
            _ if !m.control_key() && !m.alt_key() => match text.filter(|t| !t.chars().any(char::is_control)) {
                Some(t) => {
                    self.query.push_str(t);
                    FindEffect::Search
                }
                None => FindEffect::None,
            },
            _ => FindEffect::None,
        }
    }

    pub fn draw(&self, r: &mut Renderer, theme: &Theme, pane: Rect, state: Option<&FindState>, scale: f32) {
        let m = r.cell_metrics();
        let (fg, bg) = (theme.foreground, theme.background);
        let mix = |t: f32| -> Rgba { [bg[0] + (fg[0] - bg[0]) * t, bg[1] + (fg[1] - bg[1]) * t, bg[2] + (fg[2] - bg[2]) * t, 1.0] };
        let pad = (8.0 * scale).round();
        let w = (m.width * 42.0).min(pane.w - pad * 2.0);
        let h = m.height + pad * 1.5;
        let x = (pane.x + pane.w - w - pad).round();
        let y = (pane.y + pad).round();
        r.rounded_rect(x, y, w, h, RADIUS * scale, mix(0.1));
        r.rounded_outline([x, y, w, h], RADIUS * scale, scale.round().max(1.0), mix(0.22));
        let ty = (y + (h - m.height) / 2.0).round();
        let tx = x + pad;

        let status = match state {
            Some(s) if s.invalid && !self.query.is_empty() => ("invalid pattern", theme.palette[1]),
            Some(s) if !s.found && !self.query.is_empty() => ("no matches", theme.palette[1]),
            _ if self.query.is_empty() => ("", mix(0.4)),
            _ => ("↑ older · ↓ newer", mix(0.45)),
        };
        let status_w = status.0.chars().count() as f32 * m.width;
        let chip = ".*";
        let chip_w = 3.0 * m.width;
        let chip_x = x + w - pad - chip_w;
        let chip_fill = if self.regex { theme.accent } else { mix(0.18) };
        r.rounded_rect(chip_x, y + 4.0 * scale, chip_w, h - 8.0 * scale, RADIUS_SMALL * scale, chip_fill);
        label(r, chip, chip_x + m.width / 2.0, ty, chip_w, if self.regex { [1.0; 4] } else { mix(0.6) }, true);
        let status_x = chip_x - pad - status_w;
        label(r, status.0, status_x, ty, status_w + m.width, status.1, false);

        let field_w = status_x - tx - pad;
        let end = if self.query.is_empty() {
            label(r, "find in scrollback…", tx, ty, field_w, mix(0.4), false);
            tx
        } else {
            label(r, &self.query, tx, ty, field_w, fg, false)
        };
        r.rect(end, ty, (2.0 * scale).round(), m.height, theme.accent);
    }
}
