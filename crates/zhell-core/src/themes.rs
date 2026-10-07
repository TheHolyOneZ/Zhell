use std::path::{Path, PathBuf};

use regex::Regex;

use crate::config::{ConfigError, ThemeSpec, parse_hex};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Zhell,
    Iterm,
    WindowsTerminal,
    Alacritty,
}

pub fn detect(path: &Path, text: &str) -> Format {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "itermcolors" | "plist" | "xml" => Format::Iterm,
        "json" | "jsonc" => Format::WindowsTerminal,
        _ if text.trim_start().starts_with("<?xml") || text.contains("<plist") => Format::Iterm,
        _ if text.trim_start().starts_with('{') => Format::WindowsTerminal,
        _ if text.lines().any(|l| l.trim_start().starts_with("[colors")) => Format::Alacritty,
        _ => Format::Zhell,
    }
}

pub fn parse_any(path: &Path, text: &str) -> Result<ThemeSpec, ConfigError> {
    let name = path.display().to_string();
    match detect(path, text) {
        Format::Zhell => ThemeSpec::parse(text, &name),
        Format::Iterm => iterm(text, path),
        Format::WindowsTerminal => windows_terminal(text, path),
        Format::Alacritty => alacritty(text, path),
    }
}

fn err(path: &Path, message: impl Into<String>) -> ConfigError {
    ConfigError::Parse { path: path.to_owned(), message: message.into() }
}

fn build(path: &Path, fg: Option<[u8; 3]>, bg: Option<[u8; 3]>, cursor: Option<[u8; 3]>, selection: Option<[u8; 3]>, ansi: [Option<[u8; 3]>; 16]) -> Result<ThemeSpec, ConfigError> {
    let missing: Vec<usize> = (0..16).filter(|&i| ansi[i].is_none()).collect();

    let mut out = [[0; 3]; 16];
    for i in 0..16 {
        out[i] = match ansi[i].or_else(|| (i >= 8).then(|| ansi[i - 8]).flatten()) {
            Some(c) => c,
            None => return Err(err(path, format!("missing colour {} (and {} more)", ANSI_NAMES[missing[0]], missing.len() - 1))),
        };
    }
    let foreground = fg.unwrap_or(out[7]);
    Ok(ThemeSpec {
        foreground,
        background: bg.unwrap_or(out[0]),
        cursor: cursor.unwrap_or(foreground),
        selection: selection.unwrap_or(out[4]),
        accent: [0x7c, 0x3a, 0xed],
        ansi: out,
    })
}

const ANSI_NAMES: [&str; 16] = [
    "black", "red", "green", "yellow", "blue", "magenta", "cyan", "white", "bright black", "bright red",
    "bright green", "bright yellow", "bright blue", "bright magenta", "bright cyan", "bright white",
];

fn iterm(text: &str, path: &Path) -> Result<ThemeSpec, ConfigError> {
    let entry = Regex::new(r"(?s)<key>\s*([^<]+?)\s*</key>\s*<dict>(.*?)</dict>").expect("regex");
    let comp = Regex::new(r"(?s)<key>\s*(Red|Green|Blue) Component\s*</key>\s*<(?:real|integer)>\s*([^<]+?)\s*</(?:real|integer)>").expect("regex");
    let (mut fg, mut bg, mut cursor, mut sel) = (None, None, None, None);
    let mut ansi = [None; 16];
    for cap in entry.captures_iter(text) {
        let mut rgb = [None::<f32>; 3];
        for c in comp.captures_iter(&cap[2]) {
            let i = match &c[1] {
                "Red" => 0,
                "Green" => 1,
                _ => 2,
            };
            rgb[i] = c[2].parse().ok();
        }
        let [Some(r), Some(g), Some(b)] = rgb else { continue };
        let to = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        let color = Some([to(r), to(g), to(b)]);
        match cap[1].trim() {
            "Foreground Color" => fg = color,
            "Background Color" => bg = color,
            "Cursor Color" => cursor = color,
            "Selection Color" => sel = color,
            name => {
                if let Some(n) = name.strip_prefix("Ansi ").and_then(|r| r.strip_suffix(" Color")).and_then(|n| n.parse::<usize>().ok())
                    && n < 16
                {
                    ansi[n] = color;
                }
            }
        }
    }
    build(path, fg, bg, cursor, sel, ansi)
}

fn strip_jsonc(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    let mut in_str = false;
    while let Some(c) = chars.next() {
        if in_str {
            out.push(c);
            if c == '\\' {
                if let Some(n) = chars.next() {
                    out.push(n);
                }
            } else if c == '"' {
                in_str = false;
            }
            continue;
        }
        match (c, chars.peek()) {
            ('"', _) => {
                in_str = true;
                out.push(c);
            }
            ('/', Some('/')) => {
                for n in chars.by_ref() {
                    if n == '\n' {
                        out.push('\n');
                        break;
                    }
                }
            }
            ('/', Some('*')) => {
                chars.next();
                let mut prev = ' ';
                for n in chars.by_ref() {
                    if prev == '*' && n == '/' {
                        break;
                    }
                    prev = n;
                }
            }
            _ => out.push(c),
        }
    }

    Regex::new(r",(\s*[}\]])").expect("regex").replace_all(&out, "$1").into_owned()
}

fn windows_terminal(text: &str, path: &Path) -> Result<ThemeSpec, ConfigError> {
    let v: serde_json::Value = serde_json::from_str(&strip_jsonc(text)).map_err(|e| err(path, e.to_string()))?;
    let scheme = if v.get("black").is_some() {
        &v
    } else {
        let schemes = v.get("schemes").and_then(|s| s.as_array()).ok_or_else(|| err(path, "no colour scheme found"))?;
        let wanted = v.pointer("/profiles/defaults/colorScheme").and_then(|s| s.as_str());
        schemes
            .iter()
            .find(|s| wanted.is_some() && s.get("name").and_then(|n| n.as_str()) == wanted)
            .or_else(|| schemes.first())
            .ok_or_else(|| err(path, "no colour scheme found"))?
    };
    let get = |k: &str| scheme.get(k).and_then(|c| c.as_str()).and_then(parse_hex);
    const KEYS: [&str; 16] = [
        "black", "red", "green", "yellow", "blue", "purple", "cyan", "white", "brightBlack", "brightRed",
        "brightGreen", "brightYellow", "brightBlue", "brightPurple", "brightCyan", "brightWhite",
    ];
    let mut ansi = [None; 16];
    for (dst, k) in ansi.iter_mut().zip(KEYS) {
        *dst = get(k).or_else(|| get(&k.replace("urple", "agenta")));
    }
    build(path, get("foreground"), get("background"), get("cursorColor"), get("selectionBackground"), ansi)
}

fn alacritty(text: &str, path: &Path) -> Result<ThemeSpec, ConfigError> {
    let v: toml::Table = toml::from_str(text).map_err(|e| err(path, e.to_string()))?;
    let colors = v.get("colors").ok_or_else(|| err(path, "no [colors] section"))?;
    let get = |table: &str, key: &str| {
        let s = colors.get(table)?.get(key)?.as_str()?;
        let hex = s.strip_prefix("0x").map(|h| format!("#{h}")).unwrap_or_else(|| s.to_owned());
        parse_hex(&hex)
    };
    const KEYS: [&str; 8] = ["black", "red", "green", "yellow", "blue", "magenta", "cyan", "white"];
    let mut ansi = [None; 16];
    for (i, k) in KEYS.iter().enumerate() {
        ansi[i] = get("normal", k);
        ansi[i + 8] = get("bright", k);
    }
    build(path, get("primary", "foreground"), get("primary", "background"), get("cursor", "cursor"), get("selection", "background"), ansi)
}

fn hex(c: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

impl ThemeSpec {
    pub fn to_toml(&self, name: &str) -> String {
        let row = |r: &[[u8; 3]]| r.iter().map(|c| format!("\"{}\"", hex(*c))).collect::<Vec<_>>().join(", ");
        format!(
            "# Zhell theme: {name}\nforeground = \"{}\"\nbackground = \"{}\"\ncursor = \"{}\"\nselection = \"{}\"\nansi = [\n  {},\n  {},\n]\n",
            hex(self.foreground),
            hex(self.background),
            hex(self.cursor),
            hex(self.selection),
            row(&self.ansi[..8]),
            row(&self.ansi[8..]),
        )
    }
}

pub fn user_dir() -> Option<PathBuf> {
    crate::config::default_path().and_then(|p| p.parent().map(|d| d.join("themes")))
}

pub fn user_themes() -> Vec<String> {
    let Some(dir) = user_dir() else { return Vec::new() };
    let mut out: Vec<String> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            (p.extension().is_some_and(|x| x == "toml")).then(|| p.file_stem()?.to_str().map(str::to_owned)).flatten()
        })
        .collect();
    out.sort();
    out
}

pub fn import(src: &Path, name: Option<&str>) -> Result<(String, PathBuf), ConfigError> {
    let text = std::fs::read_to_string(src).map_err(|source| ConfigError::Io { path: src.to_owned(), source })?;
    let spec = parse_any(src, &text)?;
    let name = name
        .map(str::to_owned)
        .or_else(|| src.file_stem().map(|s| s.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "imported".into());
    let name: String = name.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c.to_ascii_lowercase() } else { '-' }).collect();
    let dir = user_dir().ok_or_else(|| err(src, "no config directory"))?;
    std::fs::create_dir_all(&dir).map_err(|source| ConfigError::Io { path: dir.clone(), source })?;
    let dst = dir.join(format!("{name}.toml"));
    std::fs::write(&dst, spec.to_toml(&name)).map_err(|source| ConfigError::Io { path: dst.clone(), source })?;
    Ok((name, dst))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn component(name: &str, r: f32, g: f32, b: f32) -> String {
        format!(
            "<key>{name}</key>\n<dict>\n<key>Alpha Component</key><real>1</real>\n<key>Blue Component</key><real>{b}</real>\n<key>Color Space</key><string>sRGB</string>\n<key>Green Component</key><real>{g}</real>\n<key>Red Component</key><real>{r}</real>\n</dict>\n"
        )
    }

    #[test]
    fn itermcolors() {
        let mut body = String::new();
        for i in 0..16 {
            body += &component(&format!("Ansi {i} Color"), i as f32 / 15.0, 0.0, 1.0);
        }
        body += &component("Background Color", 0.1, 0.1, 0.1);
        body += &component("Foreground Color", 0.9, 0.9, 0.9);
        let text = format!("<?xml version=\"1.0\"?>\n<plist version=\"1.0\">\n<dict>\n{body}</dict>\n</plist>");
        let t = parse_any(Path::new("x.itermcolors"), &text).unwrap();
        assert_eq!(t.background, [26, 26, 26]);
        assert_eq!(t.foreground, [230, 230, 230]);
        assert_eq!(t.ansi[15], [255, 0, 255]);
        assert_eq!(t.ansi[0], [0, 0, 255]);
        assert_eq!(t.cursor, t.foreground);
    }

    #[test]
    fn windows_terminal_settings_with_comments() {
        let text = r##"{
            // My settings
            "profiles": { "defaults": { "colorScheme": "Two" } },
            "schemes": [
                { "name": "One", "black": "#000000", "red": "#110000", "green": "#001100", "yellow": "#111100",
                  "blue": "#000011", "purple": "#110011", "cyan": "#001111", "white": "#eeeeee" },
                { "name": "Two", /* the one */ "foreground": "#abcdef", "background": "#123456",
                  "black": "#000000", "red": "#ff0000", "green": "#00ff00", "yellow": "#ffff00",
                  "blue": "#0000ff", "purple": "#ff00ff", "cyan": "#00ffff", "white": "#ffffff",
                  "brightBlack": "#808080", "brightRed": "#ff8080", "brightGreen": "#80ff80", "brightYellow": "#ffff80",
                  "brightBlue": "#8080ff", "brightPurple": "#ff80ff", "brightCyan": "#80ffff", "brightWhite": "#ffffff",
                  "cursorColor": "#ff00aa", "selectionBackground": "#333333", },
            ],
        }"##;
        let t = parse_any(Path::new("settings.json"), text).unwrap();
        assert_eq!(t.foreground, [0xab, 0xcd, 0xef]);
        assert_eq!(t.ansi[13], [0xff, 0x80, 0xff]);
        assert_eq!(t.cursor, [0xff, 0x00, 0xaa]);
        assert_eq!(t.selection, [0x33; 3]);

        let one = r##"{ "name": "One", "black": "#000000", "red": "#110000", "green": "#001100", "yellow": "#111100",
                  "blue": "#000011", "purple": "#110011", "cyan": "#001111", "white": "#eeeeee" }"##;
        let t = parse_any(Path::new("one.json"), one).unwrap();
        assert_eq!(t.ansi[9], [0x11, 0, 0]);
        assert_eq!(t.background, [0, 0, 0]);
    }

    #[test]
    fn alacritty_toml() {
        let text = r##"
[colors.primary]
background = "0x1e1e2e"
foreground = "#cdd6f4"
[colors.cursor]
cursor = "#f5e0dc"
text = "#1e1e2e"
[colors.normal]
black = "#45475a"
red = "#f38ba8"
green = "#a6e3a1"
yellow = "#f9e2af"
blue = "#89b4fa"
magenta = "#f5c2e7"
cyan = "#94e2d5"
white = "#bac2de"
[colors.bright]
black = "#585b70"
red = "#f38ba8"
green = "#a6e3a1"
yellow = "#f9e2af"
blue = "#89b4fa"
magenta = "#f5c2e7"
cyan = "#94e2d5"
white = "#a6adc8"
"##;
        let t = parse_any(Path::new("catppuccin.toml"), text).unwrap();
        assert_eq!(t.background, [0x1e, 0x1e, 0x2e]);
        assert_eq!(t.cursor, [0xf5, 0xe0, 0xdc]);
        assert_eq!(t.ansi[15], [0xa6, 0xad, 0xc8]);

        let again = ThemeSpec::parse(&t.to_toml("x"), "x").unwrap();
        assert_eq!(again, t);
    }

    #[test]
    fn missing_colours_are_named() {
        let e = parse_any(Path::new("a.json"), r##"{"black": "#000000"}"##).unwrap_err().to_string();
        assert!(e.contains("missing colour red"), "{e}");
    }

    #[test]
    fn jsonc_keeps_slashes_in_strings() {
        assert_eq!(strip_jsonc(r#"{"a": "http://x" // c
}"#), "{\"a\": \"http://x\" \n}");
    }
}
