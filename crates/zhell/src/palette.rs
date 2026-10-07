use winit::keyboard::{Key, NamedKey};
use zhell_core::keys::Action;
use zhell_core::layout::Rect;
use zhell_render::{GlyphStyle, Renderer};

use crate::draw::label;
use crate::theme::{RADIUS, Rgba, Theme};

#[derive(Clone, Debug, PartialEq)]
pub enum ItemKind {
    Action(Action),
    Theme(String),

    Command(String),
    SaveWorkspace,

    Ssh(String, Vec<String>),

    Program(String, String, Vec<String>, Option<String>),
}

#[derive(Clone, Debug)]
pub struct Item {
    pub title: String,

    pub hint: String,
    pub kind: ItemKind,
}

pub enum PaletteEffect {
    None,
    Run(ItemKind),
    Close,
}

pub struct Palette {
    query: String,
    items: Vec<Item>,

    shown: Vec<(usize, Vec<usize>)>,
    selected: usize,
    scroll: usize,
    rows: usize,
    list: Rect,
    row_h: f32,

    pub req: u32,
}

impl Palette {
    pub fn new(items: Vec<Item>) -> Self {
        let mut p = Self {
            query: String::new(),
            items,
            shown: Vec::new(),
            selected: 0,
            scroll: 0,
            rows: 10,
            list: Rect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 },
            row_h: 1.0,
            req: 0,
        };
        p.filter();
        p
    }

    pub fn add_items(&mut self, items: impl IntoIterator<Item = Item>) {
        self.items.extend(items);
        self.filter();
    }

    fn filter(&mut self) {
        let mut scored: Vec<(i32, usize, Vec<usize>)> = self
            .items
            .iter()
            .enumerate()
            .filter_map(|(i, it)| zhell_core::fuzzy::score(&self.query, &it.title).map(|(s, pos)| (s, i, pos)))
            .collect();

        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        self.shown = scored.into_iter().map(|(_, i, pos)| (i, pos)).collect();
        self.selected = 0;
        self.scroll = 0;
    }

    fn select(&mut self, i: usize) {
        if self.shown.is_empty() {
            return;
        }
        self.selected = i.min(self.shown.len() - 1);
        if self.selected < self.scroll {
            self.scroll = self.selected;
        } else if self.selected >= self.scroll + self.rows {
            self.scroll = self.selected + 1 - self.rows;
        }
    }

    fn run_selected(&self) -> PaletteEffect {
        match self.shown.get(self.selected) {
            Some((i, _)) => PaletteEffect::Run(self.items[*i].kind.clone()),
            None => PaletteEffect::None,
        }
    }

    pub fn key(&mut self, key: &Key, text: Option<&str>, ctrl: bool) -> PaletteEffect {
        match key {
            Key::Named(NamedKey::Escape) => PaletteEffect::Close,
            Key::Named(NamedKey::Enter) => self.run_selected(),
            Key::Named(NamedKey::ArrowDown) => {
                self.select(self.selected + 1);
                PaletteEffect::None
            }
            Key::Named(NamedKey::ArrowUp) => {
                self.select(self.selected.saturating_sub(1));
                PaletteEffect::None
            }
            Key::Named(NamedKey::PageDown) => {
                self.select(self.selected + self.rows);
                PaletteEffect::None
            }
            Key::Named(NamedKey::PageUp) => {
                self.select(self.selected.saturating_sub(self.rows));
                PaletteEffect::None
            }
            Key::Named(NamedKey::Backspace) => {
                self.query.pop();
                self.filter();
                PaletteEffect::None
            }
            Key::Character(c) if ctrl && c.eq_ignore_ascii_case("u") => {
                self.query.clear();
                self.filter();
                PaletteEffect::None
            }
            _ if !ctrl => {
                if let Some(t) = text.filter(|t| !t.chars().any(char::is_control)) {
                    self.query.push_str(t);
                    self.filter();
                }
                PaletteEffect::None
            }
            _ => PaletteEffect::None,
        }
    }

    pub fn click(&mut self, x: f32, y: f32) -> PaletteEffect {
        if !self.list.contains(x, y) {
            return PaletteEffect::Close;
        }
        let i = self.scroll + ((y - self.list.y) / self.row_h) as usize;
        if i >= self.shown.len() {
            return PaletteEffect::None;
        }
        self.selected = i;
        self.run_selected()
    }

    pub fn hover(&mut self, x: f32, y: f32) -> bool {
        if !self.list.contains(x, y) {
            return false;
        }
        let i = self.scroll + ((y - self.list.y) / self.row_h) as usize;
        if i < self.shown.len() && i != self.selected {
            self.selected = i;
            return true;
        }
        false
    }

    pub fn wheel(&mut self, lines: i32) {
        let max = self.shown.len().saturating_sub(self.rows);
        self.scroll = (self.scroll as i64 - lines as i64).clamp(0, max as i64) as usize;
    }

    pub fn draw(&mut self, r: &mut Renderer, theme: &Theme, area: Rect, scale: f32) {
        let m = r.cell_metrics();
        let fg = theme.foreground;
        let bg = theme.background;
        let mix = |t: f32| -> Rgba { [bg[0] + (fg[0] - bg[0]) * t, bg[1] + (fg[1] - bg[1]) * t, bg[2] + (fg[2] - bg[2]) * t, 1.0] };
        let pad = (10.0 * scale).round();
        let radius = RADIUS * scale;

        r.rect(area.x, area.y, area.w, area.h, [bg[0], bg[1], bg[2], 0.45]);
        let w = (area.w * 0.6).clamp(m.width * 40.0, m.width * 90.0).min(area.w - 2.0 * pad);
        self.row_h = (m.height + 10.0 * scale).round();
        let max_rows = (((area.h * 0.6) / self.row_h) as usize).max(3);
        self.rows = self.shown.len().clamp(1, max_rows);
        let h = pad * 2.0 + m.height + 12.0 * scale + pad + self.rows as f32 * self.row_h + pad;
        let panel = Rect { x: (area.x + (area.w - w) / 2.0).round(), y: (area.y + area.h * 0.12).round(), w, h };
        let edge = scale.round().max(1.0);
        r.rounded_rect(panel.x, panel.y, panel.w, panel.h, radius, mix(0.07));
        r.rounded_outline([panel.x, panel.y, panel.w, panel.h], radius, edge, mix(0.18));

        let fx = panel.x + pad;
        let fy = panel.y + pad;
        let fw = panel.w - 2.0 * pad;
        let fh = m.height + 12.0 * scale;
        r.rounded_rect(fx, fy, fw, fh, radius, mix(0.12));
        let ty = (fy + (fh - m.height) / 2.0).round();
        let tx = fx + 10.0 * scale;
        let end = if self.query.is_empty() {
            label(r, "Type a command, theme or starred command…", tx, ty, fw, mix(0.4), false);
            tx
        } else {
            label(r, &self.query, tx, ty, fw - 20.0 * scale, fg, false)
        };
        r.rect(end, ty, (2.0 * scale).round(), m.height, theme.accent);

        let ly = fy + fh + pad;
        self.list = Rect { x: fx, y: ly, w: fw, h: self.rows as f32 * self.row_h };
        if self.shown.is_empty() {
            label(r, "nothing matches", tx, ly + (self.row_h - m.height) / 2.0, fw, mix(0.4), false);
        }
        let mut buf = [0u8; 4];
        for (row, (idx, pos)) in self.shown.iter().enumerate().skip(self.scroll).take(self.rows) {
            let it = &self.items[*idx];
            let y = ly + (row - self.scroll) as f32 * self.row_h;
            if row == self.selected {
                r.rounded_rect(fx, y, fw, self.row_h - 2.0 * scale, radius, mix(0.15));
            }
            let ty = (y + (self.row_h - 2.0 * scale - m.height) / 2.0).round();

            let hint_w = it.hint.chars().count() as f32 * m.width;
            let max_x = fx + fw - hint_w - 24.0 * scale;
            let mut cx = tx;
            for (ci, ch) in it.title.chars().enumerate() {
                if cx + m.width > max_x {
                    break;
                }
                let hit = pos.contains(&ci);
                let color = if hit { theme.accent } else { fg };
                r.glyph(cx, ty, ch.encode_utf8(&mut buf), GlyphStyle { bold: hit, italic: false }, color);
                cx += m.width;
            }
            label(r, &it.hint, fx + fw - hint_w - 10.0 * scale, ty, hint_w + m.width, mix(0.45), false);
        }
    }
}
