use winit::event::KeyEvent;
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::platform::modifier_supplement::KeyEventExtModifierSupplement;
use zhell_core::keys::{self, KeyCombo};
use zhell_proto::mode;

fn named_key_name(k: NamedKey) -> Option<String> {
    Some(match k {
        NamedKey::Tab => "tab".into(),
        NamedKey::Enter => "enter".into(),
        NamedKey::Escape => "escape".into(),
        NamedKey::Backspace => "backspace".into(),
        NamedKey::Space => " ".into(),
        NamedKey::Insert => "insert".into(),
        NamedKey::Delete => "delete".into(),
        NamedKey::Home => "home".into(),
        NamedKey::End => "end".into(),
        NamedKey::PageUp => "pageup".into(),
        NamedKey::PageDown => "pagedown".into(),
        NamedKey::ArrowLeft => "left".into(),
        NamedKey::ArrowRight => "right".into(),
        NamedKey::ArrowUp => "up".into(),
        NamedKey::ArrowDown => "down".into(),
        NamedKey::F1 => "f1".into(),
        NamedKey::F2 => "f2".into(),
        NamedKey::F3 => "f3".into(),
        NamedKey::F4 => "f4".into(),
        NamedKey::F5 => "f5".into(),
        NamedKey::F6 => "f6".into(),
        NamedKey::F7 => "f7".into(),
        NamedKey::F8 => "f8".into(),
        NamedKey::F9 => "f9".into(),
        NamedKey::F10 => "f10".into(),
        NamedKey::F11 => "f11".into(),
        NamedKey::F12 => "f12".into(),
        _ => return None,
    })
}

fn key_name(key: &Key) -> Option<String> {
    match key {
        Key::Character(c) => Some(c.to_lowercase()),
        Key::Named(n) => named_key_name(*n),
        _ => None,
    }
}

pub fn combos(event: &KeyEvent, m: ModifiersState) -> Vec<KeyCombo> {
    let mut mods = 0;
    if m.control_key() {
        mods |= keys::CTRL;
    }
    if m.shift_key() {
        mods |= keys::SHIFT;
    }
    if m.alt_key() {
        mods |= keys::ALT;
    }
    if m.super_key() {
        mods |= keys::SUPER;
    }
    let mut out = Vec::new();
    if let Some(base) = key_name(&event.key_without_modifiers()) {
        out.push(KeyCombo::new(mods, &base));
    }
    if let Some(shifted) = key_name(&event.logical_key)
        && mods & keys::SHIFT != 0
        && out.first().is_none_or(|c| c.key != shifted)
    {
        out.push(KeyCombo::new(mods & !keys::SHIFT, &shifted));
    }
    out
}

fn mod_param(m: ModifiersState) -> u8 {
    1 + m.shift_key() as u8 + 2 * m.alt_key() as u8 + 4 * m.control_key() as u8
}

fn csi_letter(letter: char, m: ModifiersState, app_cursor: bool) -> Vec<u8> {
    let p = mod_param(m);
    if p > 1 {
        format!("\x1b[1;{p}{letter}").into_bytes()
    } else if app_cursor {
        format!("\x1bO{letter}").into_bytes()
    } else {
        format!("\x1b[{letter}").into_bytes()
    }
}

fn csi_tilde(n: u8, m: ModifiersState) -> Vec<u8> {
    let p = mod_param(m);
    if p > 1 { format!("\x1b[{n};{p}~").into_bytes() } else { format!("\x1b[{n}~").into_bytes() }
}

fn ss3_fkey(letter: char, m: ModifiersState) -> Vec<u8> {
    let p = mod_param(m);
    if p > 1 { format!("\x1b[1;{p}{letter}").into_bytes() } else { format!("\x1bO{letter}").into_bytes() }
}

fn with_alt(mut bytes: Vec<u8>, m: ModifiersState) -> Vec<u8> {
    if m.alt_key() {
        bytes.insert(0, 0x1b);
    }
    bytes
}

pub fn encode_key(key: &Key, text: Option<&str>, m: ModifiersState, modes: u32) -> Option<Vec<u8>> {
    let app_cursor = modes & mode::APP_CURSOR != 0;
    let out = match key {
        Key::Named(named) => match named {
            NamedKey::Enter => with_alt(b"\r".to_vec(), m),
            NamedKey::Backspace => {
                with_alt(if m.control_key() { vec![0x08] } else { vec![0x7f] }, m)
            }
            NamedKey::Tab => {
                if m.shift_key() { b"\x1b[Z".to_vec() } else { with_alt(b"\t".to_vec(), m) }
            }
            NamedKey::Escape => with_alt(b"\x1b".to_vec(), m),
            NamedKey::Space => {
                if m.control_key() { vec![0] } else { with_alt(b" ".to_vec(), m) }
            }
            NamedKey::ArrowUp => csi_letter('A', m, app_cursor),
            NamedKey::ArrowDown => csi_letter('B', m, app_cursor),
            NamedKey::ArrowRight => csi_letter('C', m, app_cursor),
            NamedKey::ArrowLeft => csi_letter('D', m, app_cursor),
            NamedKey::Home => csi_letter('H', m, app_cursor),
            NamedKey::End => csi_letter('F', m, app_cursor),
            NamedKey::Insert => csi_tilde(2, m),
            NamedKey::Delete => csi_tilde(3, m),
            NamedKey::PageUp => csi_tilde(5, m),
            NamedKey::PageDown => csi_tilde(6, m),
            NamedKey::F1 => ss3_fkey('P', m),
            NamedKey::F2 => ss3_fkey('Q', m),
            NamedKey::F3 => ss3_fkey('R', m),
            NamedKey::F4 => ss3_fkey('S', m),
            NamedKey::F5 => csi_tilde(15, m),
            NamedKey::F6 => csi_tilde(17, m),
            NamedKey::F7 => csi_tilde(18, m),
            NamedKey::F8 => csi_tilde(19, m),
            NamedKey::F9 => csi_tilde(20, m),
            NamedKey::F10 => csi_tilde(21, m),
            NamedKey::F11 => csi_tilde(23, m),
            NamedKey::F12 => csi_tilde(24, m),
            _ => return text.filter(|t| !t.is_empty()).map(|t| with_alt(t.as_bytes().to_vec(), m)),
        },
        Key::Character(c) => {
            if m.control_key() {
                let ch = c.chars().next()?;
                let ctrl = match ch.to_ascii_lowercase() {
                    l @ 'a'..='z' => l as u8 - b'a' + 1,
                    '@' | '2' | ' ' => 0,
                    '[' | '3' => 0x1b,
                    '\\' | '4' => 0x1c,
                    ']' | '5' => 0x1d,
                    '^' | '6' => 0x1e,
                    '_' | '7' | '-' | '/' => 0x1f,
                    '8' | '?' => 0x7f,
                    _ => return text.map(|t| with_alt(t.as_bytes().to_vec(), m)),
                };
                with_alt(vec![ctrl], m)
            } else {
                let t = text.unwrap_or(c.as_str());
                with_alt(t.as_bytes().to_vec(), m)
            }
        }
        _ => return None,
    };
    Some(out)
}

pub mod kitty {
    pub const DISAMBIGUATE: u32 = 1 << 18;
    pub const EVENT_TYPES: u32 = 1 << 19;
    pub const ALTERNATE_KEYS: u32 = 1 << 20;
    pub const ALL_AS_ESCAPES: u32 = 1 << 21;
    pub const ASSOCIATED_TEXT: u32 = 1 << 22;
    pub const ANY: u32 = DISAMBIGUATE | EVENT_TYPES | ALTERNATE_KEYS | ALL_AS_ESCAPES | ASSOCIATED_TEXT;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyAction {
    Press,
    Repeat,
    Release,
}

pub struct KittyKey<'a> {
    pub logical: &'a Key,

    pub base: &'a Key,
    pub text: Option<&'a str>,
    pub mods: ModifiersState,
    pub action: KeyAction,
}

enum Functional {
    U(u32),

    Tilde(u32),

    Letter(char),
}

fn functional(k: NamedKey) -> Option<Functional> {
    use Functional::*;
    Some(match k {
        NamedKey::Escape => U(27),
        NamedKey::Enter => U(13),
        NamedKey::Tab => U(9),
        NamedKey::Backspace => U(127),
        NamedKey::Insert => Tilde(2),
        NamedKey::Delete => Tilde(3),
        NamedKey::ArrowLeft => Letter('D'),
        NamedKey::ArrowRight => Letter('C'),
        NamedKey::ArrowUp => Letter('A'),
        NamedKey::ArrowDown => Letter('B'),
        NamedKey::PageUp => Tilde(5),
        NamedKey::PageDown => Tilde(6),
        NamedKey::Home => Letter('H'),
        NamedKey::End => Letter('F'),
        NamedKey::CapsLock => U(57358),
        NamedKey::ScrollLock => U(57359),
        NamedKey::NumLock => U(57360),
        NamedKey::PrintScreen => U(57361),
        NamedKey::Pause => U(57362),
        NamedKey::ContextMenu => U(57363),
        NamedKey::F1 => Letter('P'),
        NamedKey::F2 => Letter('Q'),
        NamedKey::F3 => Tilde(13),
        NamedKey::F4 => Letter('S'),
        NamedKey::F5 => Tilde(15),
        NamedKey::F6 => Tilde(17),
        NamedKey::F7 => Tilde(18),
        NamedKey::F8 => Tilde(19),
        NamedKey::F9 => Tilde(20),
        NamedKey::F10 => Tilde(21),
        NamedKey::F11 => Tilde(23),
        NamedKey::F12 => Tilde(24),
        NamedKey::Shift => U(57441),
        NamedKey::Control => U(57442),
        NamedKey::Alt => U(57443),
        NamedKey::Super => U(57444),
        NamedKey::Space => U(32),
        _ => return None,
    })
}

fn is_modifier(k: &Key) -> bool {
    matches!(k, Key::Named(NamedKey::Shift | NamedKey::Control | NamedKey::Alt | NamedKey::Super))
}

pub fn encode_kitty(k: &KittyKey<'_>, modes: u32) -> Option<Vec<u8>> {
    let all_escapes = modes & kitty::ALL_AS_ESCAPES != 0;
    let event_types = modes & kitty::EVENT_TYPES != 0;
    let m = k.mods;
    let mods_value = 1 + m.shift_key() as u32 + 2 * m.alt_key() as u32 + 4 * m.control_key() as u32 + 8 * m.super_key() as u32;

    if k.action == KeyAction::Release && !event_types {
        return None;
    }

    if is_modifier(k.logical) && !all_escapes {
        return None;
    }

    let event_suffix = match (event_types, k.action) {
        (true, KeyAction::Repeat) => ":2",
        (true, KeyAction::Release) => ":3",
        _ => "",
    };
    let mods_field = |force: bool| -> String {
        if mods_value > 1 || !event_suffix.is_empty() || force {
            format!("{mods_value}{event_suffix}")
        } else {
            String::new()
        }
    };

    match k.logical {
        Key::Named(named) => {
            let f = functional(*named)?;

            if let Functional::U(code @ (13 | 9 | 127 | 32)) = f {
                if !all_escapes && mods_value == 1 {
                    if k.action == KeyAction::Release {
                        return None;
                    }
                    return Some(match code {
                        13 => b"\r".to_vec(),
                        9 => b"\t".to_vec(),
                        127 => vec![0x7f],
                        _ => b" ".to_vec(),
                    });
                }
                if !all_escapes && k.action == KeyAction::Release {
                    return None;
                }
            }
            Some(match f {
                Functional::U(code) => {
                    let mf = mods_field(false);
                    if mf.is_empty() { format!("\x1b[{code}u") } else { format!("\x1b[{code};{mf}u") }
                }
                Functional::Tilde(n) => {
                    let mf = mods_field(false);
                    if mf.is_empty() { format!("\x1b[{n}~") } else { format!("\x1b[{n};{mf}~") }
                }
                Functional::Letter(c) => {
                    let mf = mods_field(false);
                    if mf.is_empty() { format!("\x1b[{c}") } else { format!("\x1b[1;{mf}{c}") }
                }
            }
            .into_bytes())
        }
        Key::Character(logical) => {
            let base_char = match k.base {
                Key::Character(b) => b.chars().next(),
                _ => None,
            }
            .or_else(|| logical.chars().next())?;
            let code = base_char.to_lowercase().next().unwrap_or(base_char) as u32;

            let text_only = !m.control_key() && !m.alt_key() && !m.super_key();
            if text_only && !all_escapes {
                if k.action == KeyAction::Release {
                    return None;
                }
                return Some(k.text.unwrap_or(logical).as_bytes().to_vec());
            }
            let mut key_field = code.to_string();
            if modes & kitty::ALTERNATE_KEYS != 0 {
                let shifted = logical.chars().next().map(|c| c as u32).filter(|&c| c != code);
                if let Some(s) = shifted {
                    key_field.push_str(&format!(":{s}"));
                }
            }
            let mut out = format!("\x1b[{key_field}");
            let mf = mods_field(false);
            let text_field = if modes & kitty::ASSOCIATED_TEXT != 0 && k.action != KeyAction::Release {
                k.text.filter(|t| !t.is_empty() && !t.chars().any(char::is_control)).map(|t| {
                    t.chars().map(|c| (c as u32).to_string()).collect::<Vec<_>>().join(":")
                })
            } else {
                None
            };
            match (mf.is_empty(), &text_field) {
                (true, None) => {}
                (true, Some(t)) => out.push_str(&format!(";1;{t}")),
                (false, None) => out.push_str(&format!(";{mf}")),
                (false, Some(t)) => out.push_str(&format!(";{mf};{t}")),
            }
            out.push('u');
            Some(out.into_bytes())
        }
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
    WheelUp,
    WheelDown,
}

pub fn encode_mouse(
    button: Option<MouseButton>,
    pressed: bool,
    motion: bool,
    col: u16,
    row: u16,
    m: ModifiersState,
    modes: u32,
) -> Option<Vec<u8>> {
    let any = mode::MOUSE_REPORT_CLICK | mode::MOUSE_DRAG | mode::MOUSE_MOTION;
    if modes & any == 0 {
        return None;
    }
    if motion {
        let allowed = (modes & mode::MOUSE_MOTION != 0)
            || (modes & mode::MOUSE_DRAG != 0 && button.is_some());
        if !allowed {
            return None;
        }
    }
    let mut code: u32 = match button {
        Some(MouseButton::Left) => 0,
        Some(MouseButton::Middle) => 1,
        Some(MouseButton::Right) => 2,
        Some(MouseButton::WheelUp) => 64,
        Some(MouseButton::WheelDown) => 65,
        None => 3,
    };
    if motion {
        code += 32;
    }
    code += 4 * m.shift_key() as u32 + 8 * m.alt_key() as u32 + 16 * m.control_key() as u32;
    let (x, y) = (col as u32 + 1, row as u32 + 1);

    if modes & mode::SGR_MOUSE != 0 {
        let end = if pressed || motion { 'M' } else { 'm' };
        return Some(format!("\x1b[<{code};{x};{y}{end}").into_bytes());
    }

    if !pressed && !motion && !matches!(button, Some(MouseButton::WheelUp | MouseButton::WheelDown)) {
        code = (code & !3) | 3;
    }
    if x > 223 || y > 223 {
        return None;
    }
    Some(vec![0x1b, b'[', b'M', 32 + code as u8, 32 + x as u8, 32 + y as u8])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k(c: &str) -> Key {
        Key::Character(c.into())
    }

    #[test]
    fn plain_and_control_chars() {
        let none = ModifiersState::empty();
        assert_eq!(encode_key(&k("a"), Some("a"), none, 0), Some(b"a".to_vec()));
        assert_eq!(encode_key(&k("c"), None, ModifiersState::CONTROL, 0), Some(vec![3]));
        assert_eq!(encode_key(&k("x"), Some("x"), ModifiersState::ALT, 0), Some(b"\x1bx".to_vec()));
        assert_eq!(encode_key(&k("ä"), Some("ä"), none, 0), Some("ä".as_bytes().to_vec()));
    }

    #[test]
    fn cursor_keys_respect_app_mode_and_modifiers() {
        let up = Key::Named(NamedKey::ArrowUp);
        let none = ModifiersState::empty();
        assert_eq!(encode_key(&up, None, none, 0), Some(b"\x1b[A".to_vec()));
        assert_eq!(encode_key(&up, None, none, mode::APP_CURSOR), Some(b"\x1bOA".to_vec()));
        assert_eq!(encode_key(&up, None, ModifiersState::CONTROL, 0), Some(b"\x1b[1;5A".to_vec()));
        assert_eq!(
            encode_key(&Key::Named(NamedKey::Delete), None, none, 0),
            Some(b"\x1b[3~".to_vec())
        );
    }

    fn kk(logical: &Key, base: &Key, text: Option<&str>, mods: ModifiersState, action: KeyAction, modes: u32) -> Option<String> {
        let k = KittyKey { logical, base, text, mods, action };
        encode_kitty(&k, modes).map(|b| String::from_utf8(b).unwrap())
    }

    #[test]
    fn kitty_disambiguate() {
        let d = kitty::DISAMBIGUATE;
        let none = ModifiersState::empty();
        let a = k("a");
        assert_eq!(kk(&a, &a, Some("a"), none, KeyAction::Press, d).as_deref(), Some("a"));
        assert_eq!(kk(&a, &a, None, ModifiersState::CONTROL, KeyAction::Press, d).as_deref(), Some("\x1b[97;5u"));
        let esc = Key::Named(NamedKey::Escape);
        assert_eq!(kk(&esc, &esc, None, none, KeyAction::Press, d).as_deref(), Some("\x1b[27u"));
        let enter = Key::Named(NamedKey::Enter);
        assert_eq!(kk(&enter, &enter, Some("\r"), none, KeyAction::Press, d).as_deref(), Some("\r"));
        assert_eq!(kk(&enter, &enter, None, ModifiersState::SHIFT, KeyAction::Press, d).as_deref(), Some("\x1b[13;2u"));
        let up = Key::Named(NamedKey::ArrowUp);
        assert_eq!(kk(&up, &up, None, none, KeyAction::Press, d).as_deref(), Some("\x1b[A"));
        assert_eq!(kk(&up, &up, None, ModifiersState::ALT, KeyAction::Press, d).as_deref(), Some("\x1b[1;3A"));

        assert_eq!(kk(&a, &a, None, ModifiersState::CONTROL, KeyAction::Release, d), None);
    }

    #[test]
    fn kitty_event_types_and_all_escapes() {
        let flags = kitty::DISAMBIGUATE | kitty::EVENT_TYPES | kitty::ALL_AS_ESCAPES;
        let none = ModifiersState::empty();
        let a = k("a");
        assert_eq!(kk(&a, &a, Some("a"), none, KeyAction::Press, flags).as_deref(), Some("\x1b[97u"));
        assert_eq!(kk(&a, &a, Some("a"), none, KeyAction::Repeat, flags).as_deref(), Some("\x1b[97;1:2u"));
        assert_eq!(kk(&a, &a, None, none, KeyAction::Release, flags).as_deref(), Some("\x1b[97;1:3u"));
        let shift = Key::Named(NamedKey::Shift);
        assert_eq!(kk(&shift, &shift, None, ModifiersState::SHIFT, KeyAction::Press, flags).as_deref(), Some("\x1b[57441;2u"));
        let f5 = Key::Named(NamedKey::F5);
        assert_eq!(kk(&f5, &f5, None, none, KeyAction::Release, flags).as_deref(), Some("\x1b[15;1:3~"));
    }

    #[test]
    fn kitty_alternate_keys_and_text() {
        let flags = kitty::DISAMBIGUATE | kitty::ALL_AS_ESCAPES | kitty::ALTERNATE_KEYS | kitty::ASSOCIATED_TEXT;
        let upper = k("A");
        let base = k("a");
        assert_eq!(
            kk(&upper, &base, Some("A"), ModifiersState::SHIFT, KeyAction::Press, flags).as_deref(),
            Some("\x1b[97:65;2;65u")
        );
    }

    #[test]
    fn sgr_and_legacy_mouse() {
        let none = ModifiersState::empty();
        let sgr = mode::MOUSE_REPORT_CLICK | mode::SGR_MOUSE;
        assert_eq!(
            encode_mouse(Some(MouseButton::Left), true, false, 4, 2, none, sgr),
            Some(b"\x1b[<0;5;3M".to_vec())
        );
        assert_eq!(
            encode_mouse(Some(MouseButton::Left), false, false, 4, 2, none, sgr),
            Some(b"\x1b[<0;5;3m".to_vec())
        );
        assert_eq!(
            encode_mouse(Some(MouseButton::Left), false, false, 0, 0, none, mode::MOUSE_REPORT_CLICK),
            Some(vec![0x1b, b'[', b'M', 35, 33, 33])
        );
        assert_eq!(encode_mouse(Some(MouseButton::Left), true, false, 0, 0, none, 0), None);

        assert_eq!(encode_mouse(None, false, true, 0, 0, none, sgr | mode::MOUSE_DRAG), None);
    }
}
