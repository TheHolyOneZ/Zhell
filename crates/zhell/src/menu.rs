use winit::keyboard::{Key, NamedKey};
use zhell_core::layout::Rect;
use zhell_render::Renderer;

use crate::draw::label;
use crate::theme::{RADIUS, Rgba, Theme};

#[derive(Clone, Debug, PartialEq)]
pub enum MenuAction {
    Copy,
    Paste,
    OpenLink,
    CopyOutput(zhell_proto::PaneId, u32),
    ToggleFold(zhell_proto::PaneId, u32),
    SaveOutput(zhell_proto::PaneId, u32, Option<String>),

    ExportHtml(zhell_proto::PaneId, u32, Option<String>),
    SplitRight,
    SplitDown,
    NewTab,
    Find,
    SearchHistory,
    Palette,
    ClosePane,

    Run(zhell_core::keys::Action),
}

pub struct Item {
    pub label: String,
    pub hint: String,
    pub action: Option<MenuAction>,
}

impl Item {
    pub fn new(label: &str, hint: String, action: MenuAction) -> Self {
        Self { label: label.into(), hint, action: Some(action) }
    }

    pub fn separator() -> Self {
        Self { label: String::new(), hint: String::new(), action: None }
    }
}

pub struct Menu {
    items: Vec<Item>,
    x: f32,
    y: f32,
    selected: Option<usize>,
    rects: Vec<Rect>,
    panel: Rect,
}

pub enum MenuResult {
    Open,
    Close,
    Run(MenuAction),
}

impl Menu {
    pub fn new(items: Vec<Item>, x: f32, y: f32) -> Self {
        Self { items, x, y, selected: None, rects: Vec::new(), panel: Rect { x, y, w: 0.0, h: 0.0 } }
    }

    fn step(&mut self, down: bool) {
        let n = self.items.len();
        let mut i = self.selected.unwrap_or(if down { n - 1 } else { 0 });
        for _ in 0..n {
            i = if down { (i + 1) % n } else { (i + n - 1) % n };
            if self.items[i].action.is_some() {
                self.selected = Some(i);
                return;
            }
        }
    }

    pub fn key(&mut self, key: &Key) -> MenuResult {
        match key {
            Key::Named(NamedKey::Escape) => MenuResult::Close,
            Key::Named(NamedKey::ArrowDown | NamedKey::Tab) => {
                self.step(true);
                MenuResult::Open
            }
            Key::Named(NamedKey::ArrowUp) => {
                self.step(false);
                MenuResult::Open
            }
            Key::Named(NamedKey::Enter | NamedKey::Space) => match self.selected.and_then(|i| self.items[i].action.clone()) {
                Some(a) => MenuResult::Run(a),
                None => MenuResult::Open,
            },
            _ => MenuResult::Open,
        }
    }

    pub fn hover(&mut self, x: f32, y: f32) -> bool {
        let i = self.rects.iter().position(|r| r.contains(x, y)).filter(|&i| self.items[i].action.is_some());
        if i != self.selected {
            self.selected = i;
            return true;
        }
        false
    }

    pub fn click(&self, x: f32, y: f32) -> MenuResult {
        if !self.panel.contains(x, y) {
            return MenuResult::Close;
        }
        match self.rects.iter().position(|r| r.contains(x, y)).and_then(|i| self.items[i].action.clone()) {
            Some(a) => MenuResult::Run(a),
            None => MenuResult::Open,
        }
    }

    pub fn draw(&mut self, r: &mut Renderer, theme: &Theme, win_w: f32, win_h: f32, scale: f32) {
        let m = r.cell_metrics();
        let (fg, bg) = (theme.foreground, theme.background);
        let mix = |t: f32| -> Rgba { [bg[0] + (fg[0] - bg[0]) * t, bg[1] + (fg[1] - bg[1]) * t, bg[2] + (fg[2] - bg[2]) * t, 1.0] };
        let pad = (6.0 * scale).round();
        let row_h = (m.height + 8.0 * scale).round();
        let sep_h = (9.0 * scale).round();
        let label_w = self.items.iter().map(|i| i.label.chars().count()).max().unwrap_or(10) as f32;
        let hint_w = self.items.iter().map(|i| i.hint.chars().count()).max().unwrap_or(0) as f32;
        let w = ((label_w + hint_w + 6.0) * m.width).round();
        let h = self.items.iter().map(|i| if i.action.is_some() { row_h } else { sep_h }).sum::<f32>() + pad * 2.0;

        let x = self.x.min(win_w - w - pad).max(pad);
        let y = if self.y + h > win_h - pad { (self.y - h).max(pad) } else { self.y };
        self.panel = Rect { x, y, w, h };
        r.rounded_rect(x, y, w, h, RADIUS * scale, mix(0.08));
        r.rounded_outline([x, y, w, h], RADIUS * scale, scale.round().max(1.0), mix(0.2));
        self.rects.clear();
        let mut cy = y + pad;
        for (i, item) in self.items.iter().enumerate() {
            if item.action.is_none() {
                r.rect(x + pad * 2.0, (cy + sep_h / 2.0).round(), w - pad * 4.0, scale.round().max(1.0), mix(0.15));
                self.rects.push(Rect { x, y: cy, w: 0.0, h: 0.0 });
                cy += sep_h;
                continue;
            }
            let rect = Rect { x: x + pad, y: cy, w: w - pad * 2.0, h: row_h };
            if self.selected == Some(i) {
                r.rounded_rect(rect.x, rect.y, rect.w, rect.h, (RADIUS - 1.0) * scale, theme.accent);
            }
            let ty = (cy + (row_h - m.height) / 2.0).round();
            let tc = if self.selected == Some(i) { [1.0; 4] } else { fg };
            label(r, &item.label, rect.x + m.width, ty, rect.w, tc, false);
            let hw = item.hint.chars().count() as f32 * m.width;
            let hc = if self.selected == Some(i) { [1.0, 1.0, 1.0, 0.75] } else { mix(0.45) };
            label(r, &item.hint, rect.x + rect.w - hw - m.width, ty, hw + m.width, hc, false);
            self.rects.push(rect);
            cy += row_h;
        }
    }
}
