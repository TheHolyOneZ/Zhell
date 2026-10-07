use regex::Regex;
use winit::keyboard::{Key, NamedKey};
use zhell_proto::{Cell, PaneId, flags};
use zhell_render::{GlyphStyle, Renderer};

use crate::theme::{RADIUS_SMALL, Theme};

const PATTERNS: &[&str] = &[
    r#"\b(?:https?|ftp|file|ssh|git)://[^\s<>"'`]+[^\s<>"'`.,;:!?)\]}]"#,

    r"\b[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}\b",

    r"(?:~|\.{1,2})?/[\w.@+~-]+(?:/[\w.@+~-]+)*/?|\b[\w.@+-]+(?:/[\w.@+~-]+)+/?",

    r"\b\d{1,3}(?:\.\d{1,3}){3}(?::\d{1,5})?\b",

    r"\b[0-9a-f]{7,64}\b",

    r"\b\d{4,}\b",
];

const ALPHABET: &[u8] = b"asdfjklghqweruiopzxcvbnmty";

pub struct Match {
    pub cells: Vec<(u16, u16)>,
    pub text: String,
    pub label: String,
}

pub struct QuickSelect {
    pub pane: PaneId,
    pub matches: Vec<Match>,
    typed: String,
}

pub enum Outcome {
    Pending,
    Cancel,

    Pick { text: String, paste: bool },
}

fn logical_lines(lines: &[Vec<Cell>]) -> Vec<(String, Vec<(u16, u16)>)> {
    let mut out: Vec<(String, Vec<(u16, u16)>)> = Vec::new();
    let mut continues = false;
    for (r, row) in lines.iter().enumerate() {
        if !continues {
            out.push((String::new(), Vec::new()));
        }
        let (text, cells) = out.last_mut().expect("pushed above");
        for (c, cell) in row.iter().enumerate() {
            if cell.flags & (flags::WIDE_CHAR_SPACER | flags::LEADING_WIDE_CHAR_SPACER) != 0 {
                continue;
            }
            text.push(cell.ch);
            cells.push((r as u16, c as u16));
            for z in &cell.zerowidth {
                text.push(*z);
                cells.push((r as u16, c as u16));
            }
        }
        continues = row.last().is_some_and(|c| c.flags & flags::WRAPLINE != 0);
    }
    out
}

fn keep(text: &str) -> bool {
    let hexish = text.len() >= 7 && text.bytes().all(|b| b.is_ascii_hexdigit());
    if hexish {
        return text.bytes().any(|b| b.is_ascii_digit());
    }
    let version = text
        .split_once('/')
        .is_some_and(|(name, ver)| !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric()) && ver.starts_with(|c: char| c.is_ascii_digit()) && ver.bytes().all(|b| b.is_ascii_digit() || b == b'.'));
    !version
}

fn labels(n: usize) -> Vec<String> {
    let a = ALPHABET.len();
    if n <= a {
        return ALPHABET[..n].iter().map(|c| (*c as char).to_string()).collect();
    }
    (0..n).map(|i| format!("{}{}", ALPHABET[(i / a) % a] as char, ALPHABET[i % a] as char)).collect()
}

impl QuickSelect {
    pub fn new(pane: PaneId, lines: &[Vec<Cell>], extra: &[String]) -> Option<Self> {
        let regexes: Vec<Regex> = extra.iter().map(String::as_str).chain(PATTERNS.iter().copied()).filter_map(|p| Regex::new(p).ok()).collect();
        let mut found: Vec<(Vec<(u16, u16)>, String)> = Vec::new();
        for (text, cells) in logical_lines(lines) {
            let starts: Vec<usize> = text.char_indices().map(|(i, _)| i).collect();
            let char_at = |byte: usize| starts.partition_point(|&s| s < byte);
            let mut taken: Vec<(usize, usize)> = Vec::new();
            for re in &regexes {
                for m in re.find_iter(&text) {
                    let (a, b) = (char_at(m.start()), char_at(m.end()));
                    if b <= a || !keep(m.as_str()) || taken.iter().any(|&(x, y)| a < y && x < b) {
                        continue;
                    }
                    taken.push((a, b));
                    let mut cs: Vec<(u16, u16)> = cells[a..b].to_vec();
                    cs.dedup();
                    found.push((cs, m.as_str().to_owned()));
                }
            }
        }
        if found.is_empty() {
            return None;
        }

        found.sort_by_key(|(c, _)| c[0]);
        let mut unique: Vec<&str> = Vec::new();
        for (_, t) in &found {
            if !unique.contains(&t.as_str()) {
                unique.push(t);
            }
        }

        let ls = labels(unique.len());
        let label_of = |t: &str| ls[unique.len() - 1 - unique.iter().position(|u| *u == t).unwrap_or(0)].clone();
        let matches = found.iter().map(|(cells, text)| Match { cells: cells.clone(), text: text.clone(), label: label_of(text) }).collect();
        Some(Self { pane, matches, typed: String::new() })
    }

    pub fn key(&mut self, key: &Key, text: Option<&str>) -> Outcome {
        if matches!(key, Key::Named(NamedKey::Escape)) {
            return Outcome::Cancel;
        }
        if matches!(key, Key::Named(NamedKey::Backspace)) {
            self.typed.pop();
            return Outcome::Pending;
        }
        let Some(ch) = text.and_then(|t| t.chars().next()).filter(|c| c.is_ascii_alphabetic()) else { return Outcome::Pending };
        let paste = ch.is_ascii_uppercase();
        self.typed.push(ch.to_ascii_lowercase());
        if let Some(m) = self.matches.iter().find(|m| m.label == self.typed) {
            return Outcome::Pick { text: m.text.clone(), paste };
        }
        if !self.matches.iter().any(|m| m.label.starts_with(&self.typed)) {
            self.typed.clear();
        }
        Outcome::Pending
    }

    pub fn draw(&self, r: &mut Renderer, theme: &Theme, origin: (f32, f32), scale: f32) {
        let m = r.cell_metrics();
        let (bg, fg, accent) = (theme.background, theme.foreground, theme.accent);
        let wash = [accent[0], accent[1], accent[2], 0.18];
        for mat in &self.matches {
            let live = mat.label.starts_with(&self.typed);
            for &(row, col) in &mat.cells {
                let (x, y) = (origin.0 + col as f32 * m.width, origin.1 + row as f32 * m.height);
                r.rect(x, y, m.width, m.height, if live { wash } else { [bg[0], bg[1], bg[2], 0.4] });
            }
            if !live {
                continue;
            }
            let (row, col) = mat.cells[0];
            let w = mat.label.len() as f32 * m.width + 4.0 * scale;
            let (x, y) = (origin.0 + col as f32 * m.width - 2.0 * scale, origin.1 + row as f32 * m.height);
            r.rounded_rect(x, y, w, m.height, RADIUS_SMALL * scale, accent);
            let mut cx = x + 2.0 * scale;
            for (i, ch) in mat.label.chars().enumerate() {
                let c = if i < self.typed.len() { [1.0, 1.0, 1.0, 0.55] } else { [1.0, 1.0, 1.0, 1.0] };
                let mut buf = [0u8; 4];
                r.glyph(cx, y, ch.encode_utf8(&mut buf), GlyphStyle { bold: true, italic: false }, c);
                cx += m.width;
            }
            let _ = fg;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(text: &[&str]) -> Vec<Vec<Cell>> {
        text.iter().map(|l| l.chars().map(|ch| Cell { ch, ..Default::default() }).collect()).collect()
    }

    #[test]
    fn finds_urls_paths_hashes() {
        let screen = rows(&[
            "see https://example.com/a?b=1. and ~/src/zhell/main.rs",
            "commit 3f9e2a1b0c ok, server 192.168.1.20:8080 pid 48213 plain words deadbeef v1.97.1 64",
        ]);
        let q = QuickSelect::new(PaneId(1), &screen, &[]).unwrap();
        let texts: Vec<&str> = q.matches.iter().map(|m| m.text.as_str()).collect();
        assert_eq!(texts, ["https://example.com/a?b=1", "~/src/zhell/main.rs", "3f9e2a1b0c", "192.168.1.20:8080", "48213"]);

        assert_eq!(q.matches.last().unwrap().label, "a");
    }

    #[test]
    fn wrapped_url_is_one_match_and_labels_pick() {
        let mut screen = rows(&["xx https://exa", "mple.org/page yy"]);
        screen[0].last_mut().unwrap().flags |= flags::WRAPLINE;
        let mut q = QuickSelect::new(PaneId(1), &screen, &[]).unwrap();
        assert_eq!(q.matches.len(), 1);
        assert_eq!(q.matches[0].text, "https://example.org/page");
        assert_eq!(q.matches[0].cells.first(), Some(&(0, 3)));
        assert_eq!(q.matches[0].cells.last(), Some(&(1, 12)));
        match q.key(&Key::Character("A".into()), Some("A")) {
            Outcome::Pick { text, paste } => assert!(paste && text == "https://example.org/page"),
            _ => panic!("expected a pick"),
        }
    }

    #[test]
    fn version_tokens_are_not_paths() {
        let screen = rows(&["HTTP/1.0 200 OK Server: SimpleHTTP/0.6 Python/3.14.7 see src/main.rs"]);
        let q = QuickSelect::new(PaneId(1), &screen, &[]).unwrap();
        let texts: Vec<&str> = q.matches.iter().map(|m| m.text.as_str()).collect();
        assert_eq!(texts, ["src/main.rs"]);
    }

    #[test]
    fn custom_patterns_come_first() {
        let screen = rows(&["ticket ZH-1234 here"]);
        let q = QuickSelect::new(PaneId(1), &screen, &[r"\bZH-\d+\b".into()]).unwrap();
        assert_eq!(q.matches[0].text, "ZH-1234");
        assert_eq!(q.matches.len(), 1);
        assert_eq!(labels(30)[0], "aa");
    }
}
