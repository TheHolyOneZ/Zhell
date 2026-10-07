use winit::keyboard::{Key, NamedKey};
use zhell_proto::{CopyCmd, SelectKind};

pub struct CopyMode {
    pub pane: zhell_proto::PaneId,

    pub selecting: Option<SelectKind>,

    pending_g: bool,
}

pub enum Effect {
    Send(CopyCmd),

    Find,
    None,
}

impl CopyMode {
    pub fn new(pane: zhell_proto::PaneId) -> Self {
        Self { pane, selecting: None, pending_g: false }
    }

    pub fn key(&mut self, key: &Key, text: Option<&str>, ctrl: bool) -> Effect {
        use CopyCmd as C;
        let ch = text.and_then(|t| t.chars().next()).filter(|c| !c.is_control());
        let g = std::mem::take(&mut self.pending_g);
        let cmd = match (key, ch) {
            (Key::Named(NamedKey::Escape), _) => match self.selecting.take() {
                Some(kind) => C::Select(kind),
                None => C::Exit,
            },
            (Key::Named(NamedKey::Enter), _) => C::Yank,
            (Key::Named(NamedKey::ArrowLeft), _) => C::Left,
            (Key::Named(NamedKey::ArrowRight), _) => C::Right,
            (Key::Named(NamedKey::ArrowUp), _) => C::Up,
            (Key::Named(NamedKey::ArrowDown), _) => C::Down,
            (Key::Named(NamedKey::Home), _) => C::LineStart,
            (Key::Named(NamedKey::End), _) => C::LineEnd,
            (Key::Named(NamedKey::PageUp), _) => C::PageUp,
            (Key::Named(NamedKey::PageDown), _) => C::PageDown,
            (_, Some(c)) if ctrl => match c.to_ascii_lowercase() {
                'u' => C::HalfPageUp,
                'd' => C::HalfPageDown,
                'b' => C::PageUp,
                'f' => C::PageDown,
                'v' => return self.select(SelectKind::Block),
                'c' => C::Exit,
                _ => return Effect::None,
            },
            (_, Some(c)) => match c {
                'h' => C::Left,
                'j' => C::Down,
                'k' => C::Up,
                'l' => C::Right,
                '0' => C::LineStart,
                '$' => C::LineEnd,
                '^' => C::FirstNonBlank,
                'w' => C::WordNext,
                'b' => C::WordPrev,
                'e' => C::WordEnd,
                'W' => C::BigWordNext,
                'B' => C::BigWordPrev,
                'E' => C::BigWordEnd,
                'H' => C::ScreenTop,
                'M' => C::ScreenMiddle,
                'L' => C::ScreenBottom,
                '{' => C::ParagraphUp,
                '}' => C::ParagraphDown,
                '%' => C::Bracket,
                'G' => C::Bottom,
                'g' if g => C::Top,
                'g' => {
                    self.pending_g = true;
                    return Effect::None;
                }
                '[' => C::PrevPrompt,
                ']' => C::NextPrompt,
                'v' => return self.select(SelectKind::Simple),
                'V' => return self.select(SelectKind::Line),
                'y' => C::Yank,
                'q' => C::Exit,
                '/' | '?' => return Effect::Find,
                _ => return Effect::None,
            },
            _ => return Effect::None,
        };
        Effect::Send(cmd)
    }

    fn select(&mut self, kind: SelectKind) -> Effect {
        self.selecting = if self.selecting == Some(kind) { None } else { Some(kind) };
        Effect::Send(CopyCmd::Select(kind))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k(c: &str) -> Key {
        Key::Character(c.into())
    }

    #[test]
    fn vi_keys() {
        let mut m = CopyMode::new(zhell_proto::PaneId(1));
        assert!(matches!(m.key(&k("w"), Some("w"), false), Effect::Send(CopyCmd::WordNext)));
        assert!(matches!(m.key(&k("g"), Some("g"), false), Effect::None));
        assert!(matches!(m.key(&k("g"), Some("g"), false), Effect::Send(CopyCmd::Top)));
        assert!(matches!(m.key(&k("v"), Some("v"), false), Effect::Send(CopyCmd::Select(SelectKind::Simple))));

        assert!(matches!(m.key(&Key::Named(NamedKey::Escape), None, false), Effect::Send(CopyCmd::Select(SelectKind::Simple))));
        assert!(matches!(m.key(&Key::Named(NamedKey::Escape), None, false), Effect::Send(CopyCmd::Exit)));
        assert!(matches!(m.key(&k("d"), Some("d"), true), Effect::Send(CopyCmd::HalfPageDown)));
    }
}
