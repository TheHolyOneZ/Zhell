use winit::keyboard::{Key, NamedKey};
use zhell_core::layout::Rect;
use zhell_render::Renderer;

use crate::draw::label;
use crate::theme::{RADIUS, Rgba, Theme};

#[derive(Clone, Debug, PartialEq)]
pub enum Confirm {
    Paste(String),
    StopPort(zhell_proto::PaneId, u16),

    RenameTab(zhell_proto::PaneId),

    CloseWindow(bool),

    SetNote(i64),

    SetTemplate(i64),

    FillTemplate { template: String, names: Vec<String>, values: Vec<String> },

    InstallRemote(zhell_proto::PaneId),

    OpenUrl(String),
}

pub struct Dialog {
    pub title: String,
    pub body: Vec<String>,
    pub confirm_label: String,
    pub on_confirm: Confirm,

    focus: usize,
    buttons: [Rect; 2],

    pub input: Option<String>,

    input_selected: bool,

    pub alt: Option<(String, Confirm)>,
    alt_rect: Rect,

    pub cancel_label: String,
}

pub enum DialogResult {
    Open,
    Cancel,
    Confirm(Confirm),
}

impl Dialog {
    pub fn new(title: impl Into<String>, body: Vec<String>, confirm_label: impl Into<String>, on_confirm: Confirm) -> Self {
        let zero = Rect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 };
        Self { title: title.into(), body, confirm_label: confirm_label.into(), on_confirm, focus: 0, buttons: [zero; 2], input: None, input_selected: false, alt: None, alt_rect: zero, cancel_label: "Cancel".into() }
    }

    pub fn with_input(mut self, initial: String) -> Self {
        self.input_selected = !initial.is_empty();
        self.input = Some(initial);
        self.focus = 1;
        self
    }

    fn result(&self, button: usize) -> DialogResult {
        if button == 1 { DialogResult::Confirm(self.on_confirm.clone()) } else { DialogResult::Cancel }
    }

    pub fn key(&mut self, key: &Key, text: Option<&str>) -> DialogResult {
        if let Some(input) = self.input.as_mut() {
            let selected = std::mem::take(&mut self.input_selected);
            match key {
                Key::Named(NamedKey::Backspace) => {
                    if selected {
                        input.clear();
                    } else {
                        input.pop();
                    }
                    return DialogResult::Open;
                }
                Key::Named(NamedKey::Enter) => return self.result(1),
                Key::Named(NamedKey::Escape) => return DialogResult::Cancel,
                Key::Named(NamedKey::ArrowLeft | NamedKey::ArrowRight | NamedKey::Home | NamedKey::End) => {
                    return DialogResult::Open;
                }
                _ => {
                    if let Some(t) = text.filter(|t| !t.chars().any(char::is_control)) {
                        if selected {
                            input.clear();
                        }
                        input.push_str(t);
                    } else if selected {
                        self.input_selected = true;
                    }
                    return DialogResult::Open;
                }
            }
        }
        match key {
            Key::Named(NamedKey::Escape) => DialogResult::Cancel,
            Key::Named(NamedKey::Enter | NamedKey::Space) => self.result(self.focus),
            Key::Named(NamedKey::Tab | NamedKey::ArrowLeft | NamedKey::ArrowRight) => {
                self.focus = 1 - self.focus;
                DialogResult::Open
            }
            _ => DialogResult::Open,
        }
    }

    pub fn default_confirm(mut self) -> Self {
        self.focus = 1;
        self
    }

    pub fn with_alt(mut self, label: impl Into<String>, action: Confirm) -> Self {
        self.alt = Some((label.into(), action));
        self
    }

    pub fn click(&self, x: f32, y: f32) -> DialogResult {
        if let Some((_, a)) = &self.alt
            && self.alt_rect.contains(x, y)
        {
            return DialogResult::Confirm(a.clone());
        }
        match self.buttons.iter().position(|b| b.contains(x, y)) {
            Some(i) => self.result(i),
            None => DialogResult::Open,
        }
    }

    pub fn draw(&mut self, r: &mut Renderer, theme: &Theme, area: Rect, scale: f32) {
        let m = r.cell_metrics();
        let (fg, bg) = (theme.foreground, theme.background);
        let mix = |t: f32| -> Rgba { [bg[0] + (fg[0] - bg[0]) * t, bg[1] + (fg[1] - bg[1]) * t, bg[2] + (fg[2] - bg[2]) * t, 1.0] };
        let pad = (16.0 * scale).round();
        let radius = RADIUS * scale;
        r.rect(area.x, area.y, area.w, area.h, [bg[0], bg[1], bg[2], 0.55]);

        let text_w = self.body.iter().chain([&self.title]).map(|l| l.chars().count()).max().unwrap_or(20).clamp(30, 64);
        let w = (text_w as f32 * m.width + pad * 2.0).min(area.w - pad * 2.0);

        let cols = (((w - pad * 2.0) / m.width) as usize).max(10);
        let mut body: Vec<String> = Vec::new();
        for para in &self.body {
            let mut line = String::new();
            for word in para.split(' ') {
                if !line.is_empty() && line.chars().count() + 1 + word.chars().count() > cols {
                    body.push(std::mem::take(&mut line));
                }
                if !line.is_empty() {
                    line.push(' ');
                }
                line.push_str(word);
            }
            body.push(line);
        }
        let btn_h = m.height + 12.0 * scale;
        let field_h = if self.input.is_some() { m.height + 14.0 * scale + pad } else { 0.0 };
        let h = pad + m.height * 1.5 + body.len() as f32 * m.height + field_h + pad + btn_h + pad;
        let p = Rect { x: (area.x + (area.w - w) / 2.0).round(), y: (area.y + (area.h - h) / 2.5).round(), w, h };
        r.rounded_rect(p.x, p.y, p.w, p.h, radius, mix(0.08));
        r.rounded_outline([p.x, p.y, p.w, p.h], radius, scale.round().max(1.0), mix(0.2));

        let mut y = p.y + pad;
        label(r, &self.title, p.x + pad, y, w - pad * 2.0, fg, true);
        y += m.height * 1.5;
        for line in &body {
            label(r, line, p.x + pad, y, w - pad * 2.0, mix(0.7), false);
            y += m.height;
        }
        if let Some(input) = &self.input {
            let fh = m.height + 14.0 * scale;
            r.rounded_rect(p.x + pad, y, w - pad * 2.0, fh, radius, mix(0.14));
            let ty = (y + (fh - m.height) / 2.0).round();
            let tx = p.x + pad + 8.0 * scale;
            if self.input_selected {
                let sw = input.chars().count() as f32 * m.width;
                let a = theme.accent;
                r.rect(tx, ty, sw, m.height, [a[0], a[1], a[2], 0.45]);
            }
            let end = label(r, input, tx, ty, w - pad * 3.0, fg, false);
            r.rect(end, ty, (2.0 * scale).round(), m.height, theme.accent);
        }

        let by = p.y + p.h - pad - btn_h;
        let mut x = p.x + p.w - pad;
        let labels = [self.cancel_label.as_str(), self.confirm_label.as_str()];
        for i in [1, 0] {
            if i == 0 && let Some((alt_label, _)) = &self.alt {
                let bw = (alt_label.chars().count() as f32 + 4.0) * m.width;
                x -= bw;
                let rect = Rect { x, y: by, w: bw, h: btn_h };
                self.alt_rect = rect;
                r.rounded_rect(rect.x, rect.y, rect.w, rect.h, radius, mix(0.16));
                let tw = alt_label.chars().count() as f32 * m.width;
                label(r, alt_label, (rect.x + (rect.w - tw) / 2.0).round(), (rect.y + (rect.h - m.height) / 2.0).round(), tw + m.width, fg, false);
                x -= 10.0 * scale;
            }
            let bw = (labels[i].chars().count() as f32 + 4.0) * m.width;
            x -= bw;
            let rect = Rect { x, y: by, w: bw, h: btn_h };
            self.buttons[i] = rect;
            let fill = if i == 1 { theme.accent } else { mix(0.16) };
            r.rounded_rect(rect.x, rect.y, rect.w, rect.h, radius, fill);
            if self.focus == i {
                r.rounded_outline([rect.x - 2.0 * scale, rect.y - 2.0 * scale, rect.w + 4.0 * scale, rect.h + 4.0 * scale], radius + 2.0 * scale, scale.round().max(1.0), mix(0.6));
            }
            let tc = if i == 1 { [1.0, 1.0, 1.0, 1.0] } else { fg };
            let tw = labels[i].chars().count() as f32 * m.width;
            label(r, labels[i], (rect.x + (rect.w - tw) / 2.0).round(), (rect.y + (rect.h - m.height) / 2.0).round(), tw + m.width, tc, i == 1);
            x -= 10.0 * scale;
        }
    }
}
