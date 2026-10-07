use winit::keyboard::{Key, ModifiersState, NamedKey};
use zhell_core::layout::Rect;
use zhell_proto::{HistoryHit, HistoryQuery};
use zhell_render::{GlyphStyle, Renderer};

use crate::draw::label;
use crate::theme::{RADIUS, RADIUS_SMALL, Rgba, Theme};

pub enum SearchEffect {
    None,

    Query(HistoryQuery),

    Preview(i64),

    Insert(String),
    CopyOutput(String),
    Star(i64, bool),

    EditNote(i64, Option<String>),

    EditTemplate(i64, String),
    Close,
}

pub struct SearchOverlay {
    pub query: String,

    cursor: usize,
    pub failed_only: bool,
    pub starred_only: bool,
    pub results: Vec<HistoryHit>,
    pub selected: usize,
    scroll: usize,

    pub req: u32,
    pub preview: Option<(i64, String)>,

    list_rows: usize,
    list_area: Rect,
    row_h: f32,
}

fn char_byte(s: &str, idx: usize) -> usize {
    s.char_indices().nth(idx).map_or(s.len(), |(b, _)| b)
}

impl SearchOverlay {
    pub fn new() -> Self {
        Self {
            query: String::new(),
            cursor: 0,
            failed_only: false,
            starred_only: false,
            results: Vec::new(),
            selected: 0,
            scroll: 0,
            req: 0,
            preview: None,
            list_rows: 10,
            list_area: Rect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 },
            row_h: 1.0,
        }
    }

    pub fn query(&self) -> HistoryQuery {
        HistoryQuery {
            text: self.query.clone(),
            failed_only: self.failed_only,
            starred_only: self.starred_only,
            limit: 500,
            ..Default::default()
        }
    }

    pub fn set_results(&mut self, hits: Vec<HistoryHit>) -> SearchEffect {
        self.results = hits;
        self.selected = 0;
        self.scroll = 0;
        self.preview = None;
        self.selected_effect()
    }

    fn selected_effect(&self) -> SearchEffect {
        match self.results.get(self.selected) {
            Some(h) if self.preview.as_ref().is_none_or(|(id, _)| *id != h.id) => SearchEffect::Preview(h.id),
            _ => SearchEffect::None,
        }
    }

    fn select(&mut self, i: usize) -> SearchEffect {
        if self.results.is_empty() {
            return SearchEffect::None;
        }
        self.selected = i.min(self.results.len() - 1);
        if self.selected < self.scroll {
            self.scroll = self.selected;
        } else if self.selected >= self.scroll + self.list_rows {
            self.scroll = self.selected + 1 - self.list_rows;
        }
        self.selected_effect()
    }

    pub fn key(&mut self, key: &Key, text: Option<&str>, m: ModifiersState) -> SearchEffect {
        let edited = |s: &mut Self| SearchEffect::Query(s.query());
        match key {
            Key::Named(NamedKey::Escape) => SearchEffect::Close,
            Key::Named(NamedKey::Enter) => match self.results.get(self.selected) {
                Some(h) => SearchEffect::Insert(h.cmd.clone()),
                None => SearchEffect::None,
            },
            Key::Named(NamedKey::ArrowDown) => self.select(self.selected + 1),
            Key::Named(NamedKey::ArrowUp) => self.select(self.selected.saturating_sub(1)),
            Key::Named(NamedKey::PageDown) => self.select(self.selected + self.list_rows),
            Key::Named(NamedKey::PageUp) => self.select(self.selected.saturating_sub(self.list_rows)),
            Key::Named(NamedKey::ArrowLeft) => {
                self.cursor = self.cursor.saturating_sub(1);
                SearchEffect::None
            }
            Key::Named(NamedKey::ArrowRight) => {
                self.cursor = (self.cursor + 1).min(self.query.chars().count());
                SearchEffect::None
            }
            Key::Named(NamedKey::Home) => {
                self.cursor = 0;
                SearchEffect::None
            }
            Key::Named(NamedKey::End) => {
                self.cursor = self.query.chars().count();
                SearchEffect::None
            }
            Key::Named(NamedKey::Backspace) if self.cursor > 0 => {
                let start = char_byte(&self.query, self.cursor - 1);
                let end = char_byte(&self.query, self.cursor);
                self.query.replace_range(start..end, "");
                self.cursor -= 1;
                edited(self)
            }
            Key::Named(NamedKey::Delete) if self.cursor < self.query.chars().count() => {
                let start = char_byte(&self.query, self.cursor);
                let end = char_byte(&self.query, self.cursor + 1);
                self.query.replace_range(start..end, "");
                edited(self)
            }
            Key::Character(c) if m.control_key() && m.shift_key() && c.eq_ignore_ascii_case("c") => {
                match &self.preview {
                    Some((_, out)) => SearchEffect::CopyOutput(out.clone()),
                    None => SearchEffect::None,
                }
            }
            Key::Character(c) if m.control_key() && c.eq_ignore_ascii_case("u") => {
                self.query.clear();
                self.cursor = 0;
                edited(self)
            }
            Key::Character(c) if m.control_key() && c.eq_ignore_ascii_case("s") => match self.results.get_mut(self.selected) {
                Some(h) => {
                    h.starred = !h.starred;
                    SearchEffect::Star(h.id, h.starred)
                }
                None => SearchEffect::None,
            },
            Key::Character(c) if m.control_key() && c.eq_ignore_ascii_case("n") => match self.results.get(self.selected) {
                Some(h) => SearchEffect::EditNote(h.id, h.note.clone()),
                None => SearchEffect::None,
            },
            Key::Character(c) if m.control_key() && c.eq_ignore_ascii_case("e") => match self.results.get(self.selected) {
                Some(h) => SearchEffect::EditTemplate(h.id, h.template.clone().unwrap_or_else(|| h.cmd.clone())),
                None => SearchEffect::None,
            },
            Key::Character(c) if m.alt_key() && c.eq_ignore_ascii_case("f") => {
                self.failed_only = !self.failed_only;
                edited(self)
            }
            Key::Character(c) if m.alt_key() && c.eq_ignore_ascii_case("s") => {
                self.starred_only = !self.starred_only;
                edited(self)
            }
            _ if !m.control_key() && !m.alt_key() => match text.filter(|t| !t.chars().any(char::is_control)) {
                Some(t) => {
                    let at = char_byte(&self.query, self.cursor);
                    self.query.insert_str(at, t);
                    self.cursor += t.chars().count();
                    edited(self)
                }
                None => SearchEffect::None,
            },
            _ => SearchEffect::None,
        }
    }

    pub fn click(&mut self, x: f32, y: f32, double: bool) -> SearchEffect {
        let a = self.list_area;
        if !a.contains(x, y) {
            return SearchEffect::None;
        }
        let i = self.scroll + ((y - a.y) / self.row_h) as usize;
        if i >= self.results.len() {
            return SearchEffect::None;
        }
        if double {
            return SearchEffect::Insert(self.results[i].cmd.clone());
        }
        self.select(i)
    }

    pub fn wheel(&mut self, lines: i32) -> SearchEffect {
        let max = self.results.len().saturating_sub(self.list_rows);
        self.scroll = (self.scroll as i64 - lines as i64).clamp(0, max as i64) as usize;
        SearchEffect::None
    }

    pub fn draw(&mut self, r: &mut Renderer, theme: &Theme, area: Rect, scale: f32) {
        let m = r.cell_metrics();
        let fg = theme.foreground;
        let bg = theme.background;
        let mix = |t: f32| -> Rgba { [bg[0] + (fg[0] - bg[0]) * t, bg[1] + (fg[1] - bg[1]) * t, bg[2] + (fg[2] - bg[2]) * t, 1.0] };
        let pad = (12.0 * scale).round();

        r.rect(area.x, area.y, area.w, area.h, [bg[0], bg[1], bg[2], 0.72]);
        let panel = Rect {
            x: area.x + (area.w * 0.04).round(),
            y: area.y + (area.h * 0.06).round(),
            w: (area.w * 0.92).round(),
            h: (area.h * 0.88).round(),
        };
        let edge = scale.round().max(1.0);
        r.rounded_rect(panel.x, panel.y, panel.w, panel.h, RADIUS * scale, mix(0.06));
        r.rounded_outline([panel.x, panel.y, panel.w, panel.h], RADIUS * scale, edge, mix(0.16));

        let fy = panel.y + pad;
        let fx = panel.x + pad;
        let field_w = panel.w - pad * 2.0;
        r.rounded_rect(fx, fy - 4.0 * scale, field_w, m.height + 8.0 * scale, RADIUS * scale, mix(0.11));
        let prompt_end = label(r, "⌕ ", fx + 6.0 * scale, fy, m.width * 2.0, theme.accent, true);
        if self.query.is_empty() {
            label(r, "search all commands and output…", prompt_end, fy, field_w, mix(0.4), false);
        } else {
            label(r, &self.query, prompt_end, fy, field_w - (prompt_end - fx), fg, false);
        }
        let cx = prompt_end + self.cursor as f32 * m.width;
        r.rect(cx, fy, (2.0 * scale).round(), m.height, theme.accent);

        let mut chip_x = fx + field_w;
        for (on, text) in [(self.starred_only, "★ starred · alt+s"), (self.failed_only, "✗ failed · alt+f")] {
            let w = (text.chars().count() as f32 + 1.0) * m.width;
            chip_x -= w + m.width;
            let c = if on { theme.accent } else { mix(0.18) };
            r.rounded_rect(chip_x, fy - 2.0 * scale, w, m.height + 4.0 * scale, RADIUS_SMALL * scale, c);
            label(r, text, chip_x + m.width / 2.0, fy, w, if on { bg } else { mix(0.7) }, false);
        }

        let top = fy + m.height + pad;
        let bottom = panel.y + panel.h - pad - m.height;
        let list_w = (panel.w * 0.48).round();
        self.row_h = m.height * 2.0 + 4.0 * scale;
        self.list_rows = (((bottom - top) / self.row_h).floor() as usize).max(1);
        self.list_area = Rect { x: fx, y: top, w: list_w - pad, h: self.list_rows as f32 * self.row_h };
        if self.results.is_empty() {
            let msg = if self.query.is_empty() { "no history yet" } else { "no matches" };
            label(r, msg, fx, top, list_w, mix(0.45), false);
        }
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64);
        for (i, h) in self.results.iter().enumerate().skip(self.scroll).take(self.list_rows) {
            let y = top + (i - self.scroll) as f32 * self.row_h;
            if i == self.selected {
                r.rounded_rect(fx - 4.0 * scale, y, list_w - pad + 4.0 * scale, self.row_h - 2.0 * scale, RADIUS * scale, mix(0.13));
                r.rounded_rect(fx - 4.0 * scale, y + 4.0 * scale, (3.0 * scale).round(), self.row_h - 10.0 * scale, 1.5 * scale, theme.accent);
            }
            let (mark, mc) = match h.exit_code {
                Some(0) => ("✓", theme.palette[2]),
                Some(_) => ("✗", theme.palette[1]),
                None => ("·", mix(0.5)),
            };
            let x = fx + 4.0 * scale;
            label(r, mark, x, y, m.width * 2.0, mc, true);
            let meta = format!("{}  {}", ago(now.saturating_sub(h.started_ms)), h.cwd.as_deref().map(short_path).unwrap_or_default());
            let meta_w = meta.chars().count() as f32 * m.width;
            let star = if h.starred { "★ " } else { "" };
            let cmd_end = label(r, star, x + m.width * 2.0, y, m.width * 2.0, theme.palette[3], false);
            label(r, &h.cmd, cmd_end, y, list_w - pad - (cmd_end - fx) - meta_w - m.width, fg, true);
            label(r, &meta, fx + list_w - pad - meta_w, y, meta_w + m.width, mix(0.45), false);
            marked(r, theme, &h.snippet, x + m.width * 2.0, y + m.height, list_w - pad - m.width * 3.0, mix(0.55));
        }

        let px = panel.x + list_w + pad;
        let pw = panel.w - list_w - pad * 2.0;
        r.rect(px - pad / 2.0, top - 4.0 * scale, edge, bottom - top, mix(0.12));
        if let Some(h) = self.results.get(self.selected) {
            let header = format!("❯ {}", h.cmd);
            label(r, &header, px, top, pw, fg, true);
            let status = match h.exit_code {
                Some(0) => format!("exit 0 · {}", fmt_ms(h.duration_ms)),
                Some(c) => format!("exit {c} · {}", fmt_ms(h.duration_ms)),
                None => fmt_ms(h.duration_ms),
            };
            label(r, &status, px, top + m.height, pw, mix(0.5), false);
            let extra = match (&h.note, &h.template) {
                (Some(n), Some(t)) => format!("✎ {n} · ⧉ {t}"),
                (Some(n), None) => format!("✎ {n}"),
                (None, Some(t)) => format!("⧉ template: {t}"),
                (None, None) => String::new(),
            };
            label(r, &extra, px, top + m.height * 2.0, pw, theme.palette[3], false);
            let body_top = top + m.height * 3.0 + 4.0 * scale;
            let lines = ((bottom - body_top) / m.height).floor().max(0.0) as usize;
            match &self.preview {
                Some((id, out)) if *id == h.id => {
                    for (j, line) in out.lines().take(lines).enumerate() {
                        label(r, line, px, body_top + j as f32 * m.height, pw, mix(0.8), false);
                    }
                }
                _ => {
                    label(r, "loading…", px, body_top, pw, mix(0.4), false);
                }
            }
        }

        let hint = "↑↓ select · enter insert · ctrl+shift+c copy output · ctrl+s star · ctrl+n note · ctrl+e template · esc close";
        label(r, hint, fx, bottom + pad / 2.0, panel.w - pad * 2.0, mix(0.4), false);
    }
}

fn marked(r: &mut Renderer, theme: &Theme, text: &str, x: f32, y: f32, max_w: f32, color: Rgba) {
    let m = r.cell_metrics();
    let mut cx = x;
    let mut hl = false;
    let mut buf = [0u8; 4];
    for ch in text.chars() {
        match ch {
            '\u{1}' => hl = true,
            '\u{2}' => hl = false,
            '\n' | '\r' | '\t' => cx += m.width,
            c => {
                if cx + m.width > x + max_w {
                    break;
                }
                if hl {
                    r.rect(cx, y, m.width, m.height, [theme.accent[0], theme.accent[1], theme.accent[2], 0.35]);
                }
                let col = if hl { theme.foreground } else { color };
                r.glyph(cx, y, c.encode_utf8(&mut buf), GlyphStyle { bold: hl, italic: false }, col);
                cx += m.width;
            }
        }
    }
}

fn short_path(p: &str) -> String {
    let home = dirs::home_dir().map(|h| h.display().to_string());
    match home.as_deref().and_then(|h| p.strip_prefix(h)) {
        Some(rest) => format!("~{rest}"),
        None => p.to_owned(),
    }
}

fn fmt_ms(ms: u64) -> String {
    if ms < 1000 { format!("{ms} ms") } else { format!("{:.1} s", ms as f64 / 1000.0) }
}

fn ago(ms: u64) -> String {
    let s = ms / 1000;
    match s {
        0..=59 => "just now".into(),
        60..=3599 => format!("{} min ago", s / 60),
        3600..=86_399 => format!("{} h ago", s / 3600),
        86_400..=172_799 => "yesterday".into(),
        _ => format!("{} days ago", s / 86_400),
    }
}
