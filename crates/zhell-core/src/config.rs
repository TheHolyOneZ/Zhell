use std::path::{Path, PathBuf};

use serde::Deserialize;

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub font: FontConfig,
    pub window: WindowConfig,
    pub shell: ShellConfig,
    pub cursor: CursorConfig,
    pub scrollback: ScrollbackConfig,
    pub links: LinksConfig,
    pub sessions: SessionsConfig,
    pub history: HistoryConfig,
    pub paste: PasteConfig,
    pub notify: NotifyConfig,
    pub background: BackgroundConfig,
    pub effects: EffectsConfig,
    pub quake: QuakeConfig,
    pub quick_select: QuickSelectConfig,

    pub profile: Vec<Profile>,

    pub ssh: Vec<crate::ssh::SshProfile>,

    pub theme: String,
    pub theme_dark: String,
    pub theme_light: String,

    pub keys: std::collections::BTreeMap<String, crate::keys::Action>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            font: FontConfig::default(),
            window: WindowConfig::default(),
            shell: ShellConfig::default(),
            cursor: CursorConfig::default(),
            scrollback: ScrollbackConfig::default(),
            links: LinksConfig::default(),
            sessions: SessionsConfig::default(),
            history: HistoryConfig::default(),
            paste: PasteConfig::default(),
            notify: NotifyConfig::default(),
            background: BackgroundConfig::default(),
            effects: EffectsConfig::default(),
            quake: QuakeConfig::default(),
            quick_select: QuickSelectConfig::default(),
            ssh: Vec::new(),
            profile: Vec::new(),
            theme: "z-dark".into(),
            theme_dark: "z-dark".into(),
            theme_light: "z-light".into(),
            keys: Default::default(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FontConfig {
    pub family: String,

    pub size: f32,

    pub line_height: f32,

    pub ligatures: bool,
}

impl Default for FontConfig {
    fn default() -> Self {
        Self { family: String::new(), size: 15.0, line_height: 1.25, ligatures: true }
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct WindowConfig {
    pub padding: f32,

    pub width: f32,
    pub height: f32,

    pub decorations: Decorations,

    pub buttons: Buttons,

    pub opacity: f32,

    pub text_background_opacity: f32,

    pub blur: bool,

    pub smooth_scroll: bool,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BackgroundConfig {
    pub image: String,
    pub fit: Fit,

    pub shader: String,

    pub dim: Option<f32>,

    pub animate: bool,
}

impl Default for BackgroundConfig {
    fn default() -> Self {
        Self { image: String::new(), fit: Fit::Cover, shader: String::new(), dim: None, animate: true }
    }
}

impl BackgroundConfig {
    pub fn is_set(&self) -> bool {
        !self.image.trim().is_empty() || !self.shader.trim().is_empty()
    }

    pub fn dim(&self) -> f32 {
        self.dim.unwrap_or(if self.shader.trim().is_empty() { 0.8 } else { 0.35 }).clamp(0.0, 1.0)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Fit {
    #[default]
    Cover,
    Contain,
    Stretch,
    Tile,
    Center,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct QuickSelectConfig {
    pub patterns: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct QuakeConfig {
    pub width: f32,

    pub height: f32,

    pub hide_on_unfocus: bool,
}

impl Default for QuakeConfig {
    fn default() -> Self {
        Self { width: 1.0, height: 0.45, hide_on_unfocus: true }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct EffectsConfig {
    pub crt: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Buttons {
    #[default]
    Minimal,

    Dots,

    None,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Decorations {
    #[default]
    Custom,
    System,
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self { padding: 8.0, width: 960.0, height: 600.0, decorations: Decorations::Custom, buttons: Buttons::Minimal, opacity: 0.94, text_background_opacity: 1.0, blur: true, smooth_scroll: true }
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ShellConfig {
    pub program: String,
    pub args: Vec<String>,

    pub env: std::collections::BTreeMap<String, String>,

    pub integration: bool,
}

impl Default for ShellConfig {
    fn default() -> Self {
        Self { program: String::new(), args: Vec::new(), env: Default::default(), integration: true }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CursorStyle {
    #[default]
    Block,
    Beam,
    Underline,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CursorConfig {
    pub style: CursorStyle,
    pub blink: bool,

    pub smooth: bool,
}

impl Default for CursorConfig {
    fn default() -> Self {
        Self { style: CursorStyle::Block, blink: false, smooth: true }
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ScrollbackConfig {
    pub lines: usize,
}

impl Default for ScrollbackConfig {
    fn default() -> Self {
        Self { lines: 100_000 }
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct NotifyConfig {
    pub enabled: bool,

    pub min_seconds: u32,
}

impl Default for NotifyConfig {
    fn default() -> Self {
        Self { enabled: true, min_seconds: 10 }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Profile {
    pub name: String,
    pub program: String,
    pub args: Vec<String>,

    pub cwd: String,
}

pub fn installed_shells() -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    #[cfg(unix)]
    if let Ok(text) = std::fs::read_to_string("/etc/shells") {
        for line in text.lines().map(str::trim).filter(|l| l.starts_with('/')) {
            let name = line.rsplit('/').next().unwrap_or(line);
            let usable = !matches!(name, "nologin" | "false" | "git-shell" | "rbash" | "sh" | "systemd-home-fallback-shell")
                && std::path::Path::new(line).exists();
            if usable && !out.iter().any(|o| o.rsplit('/').next() == Some(name)) {
                out.push(line.to_owned());
            }
        }
    }
    #[cfg(windows)]
    for exe in ["pwsh.exe", "powershell.exe", "cmd.exe", "wsl.exe"] {
        out.push(exe.to_owned());
    }
    out
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PasteConfig {
    pub guard: bool,

    pub warn_multiline: bool,

    pub auto_screen_share: bool,
}

impl Default for PasteConfig {
    fn default() -> Self {
        Self { guard: true, warn_multiline: true, auto_screen_share: true }
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct HistoryConfig {
    pub enabled: bool,

    pub path: String,

    pub max_size_mb: u64,

    pub max_age_days: u32,

    pub exclude_dirs: Vec<String>,

    pub encrypt: bool,
}

impl Default for HistoryConfig {
    fn default() -> Self {
        Self { enabled: true, path: String::new(), max_size_mb: 2048, max_age_days: 0, exclude_dirs: Vec::new(), encrypt: false }
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SessionsConfig {
    pub daemon: bool,

    pub keep_alive: bool,

    pub restore_after_reboot: bool,
}

impl Default for SessionsConfig {
    fn default() -> Self {
        Self { daemon: true, keep_alive: true, restore_after_reboot: true }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LinksConfig {
    pub editor: String,
}

impl LinksConfig {
    pub fn editor_command(&self, file: &str, line: Option<u32>, col: Option<u32>) -> Option<Vec<String>> {
        let line = line.unwrap_or(1).to_string();
        let col = col.unwrap_or(1).to_string();
        let parts: Vec<String> = self
            .editor
            .split_whitespace()
            .map(|p| p.replace("{file}", file).replace("{line}", &line).replace("{col}", &col))
            .collect();
        (!parts.is_empty()).then_some(parts)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("cannot read {path}: {source}")]
    Io { path: PathBuf, source: std::io::Error },
    #[error("{path}: {message}")]
    Parse { path: PathBuf, message: String },
    #[error("unknown theme {0:?} (built-in: {list})", list = BUILTIN_THEMES.iter().map(|t| t.0).collect::<Vec<_>>().join(", "))]
    UnknownTheme(String),
    #[error("theme {name}: invalid colour {value:?} (expected #rrggbb)")]
    BadColor { name: String, value: String },
}

pub const DEFAULT_TEMPLATE: &str = include_str!("../../../assets/zhell.default.toml");

impl ConfigError {
    pub fn short(&self) -> String {
        match self {
            ConfigError::Parse { path, message } => {
                let file = path.file_name().map_or_else(|| path.display().to_string(), |f| f.to_string_lossy().into_owned());

                let line = message.split("at line ").nth(1).and_then(|r| r.split(',').next()).map(str::trim);
                let reason = message.lines().map(str::trim).rfind(|l| !l.is_empty() && !l.starts_with('|') && !l.contains(" | ") && !l.starts_with("TOML parse error")).unwrap_or(message);
                match line {
                    Some(n) => format!("{file} line {n}: {reason}"),
                    None => format!("{file}: {reason}"),
                }
            }
            other => other.to_string(),
        }
    }
}

pub fn default_path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("ZHELL_CONFIG") {
        return Some(PathBuf::from(p));
    }
    dirs::config_dir().map(|d| d.join("zhell").join("zhell.toml"))
}

impl Config {
    pub fn parse(text: &str, path: &Path) -> Result<Self, ConfigError> {
        let cfg: Config = toml::from_str(text)
            .map_err(|e| ConfigError::Parse { path: path.to_owned(), message: e.to_string() })?;
        crate::keys::Keymap::with_overrides(&cfg.keys)
            .map_err(|e| ConfigError::Parse { path: path.to_owned(), message: format!("[keys] {e}") })?;
        Ok(cfg.sanitized())
    }

    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse(&text, path),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(source) => Err(ConfigError::Io { path: path.to_owned(), source }),
        }
    }

    fn sanitized(mut self) -> Self {
        self.font.size = self.font.size.clamp(4.0, 200.0);
        self.font.line_height = self.font.line_height.clamp(0.8, 3.0);
        self.window.padding = self.window.padding.clamp(0.0, 200.0);
        self.window.width = self.window.width.max(100.0);
        self.window.height = self.window.height.max(60.0);
        self.window.opacity = self.window.opacity.clamp(0.3, 1.0);
        self.quake.width = self.quake.width.clamp(0.2, 1.0);
        self.window.text_background_opacity = self.window.text_background_opacity.clamp(0.0, 1.0);
        self.quake.height = self.quake.height.clamp(0.1, 1.0);
        self.scrollback.lines = self.scrollback.lines.min(1_000_000);
        self
    }

    pub fn load_theme_for(&self, config_path: Option<&Path>, dark: bool) -> Result<ThemeSpec, ConfigError> {
        if self.theme == "auto" {
            let theme = if dark { self.theme_dark.clone() } else { self.theme_light.clone() };
            return Config { theme, ..self.clone() }.load_theme(config_path);
        }
        self.load_theme(config_path)
    }

    pub fn load_theme(&self, config_path: Option<&Path>) -> Result<ThemeSpec, ConfigError> {
        if let Some((_, text)) = BUILTIN_THEMES.iter().find(|(n, _)| *n == self.theme) {
            return ThemeSpec::parse(text, &self.theme);
        }
        let candidate = PathBuf::from(expand_tilde(&self.theme));
        let dir = config_path.and_then(Path::parent);
        let path = match (candidate.is_relative(), dir) {
            (true, Some(dir)) => dir.join(&candidate),
            _ => candidate.clone(),
        };

        let named = (candidate.components().count() == 1 && candidate.extension().is_none())
            .then(|| dir.map(|d| d.join("themes")).or_else(crate::themes::user_dir))
            .flatten()
            .map(|d| d.join(format!("{}.toml", self.theme)))
            .filter(|p| p.exists());
        let path = named.unwrap_or(path);
        if path.extension().is_some() || path.exists() {
            let text = std::fs::read_to_string(&path)
                .map_err(|source| ConfigError::Io { path: path.clone(), source })?;
            return crate::themes::parse_any(&path, &text);
        }
        Err(ConfigError::UnknownTheme(self.theme.clone()))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ThemeSpec {
    pub foreground: [u8; 3],
    pub background: [u8; 3],
    pub cursor: [u8; 3],
    pub selection: [u8; 3],

    pub accent: [u8; 3],

    pub ansi: [[u8; 3]; 16],
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ThemeFile {
    foreground: String,
    background: String,
    cursor: Option<String>,
    selection: Option<String>,
    accent: Option<String>,
    ansi: Vec<String>,
}

pub fn expand_tilde(p: &str) -> String {
    match (p.strip_prefix("~/"), dirs::home_dir()) {
        (Some(rest), Some(home)) => home.join(rest).display().to_string(),
        _ => p.to_owned(),
    }
}

pub fn parse_hex(s: &str) -> Option<[u8; 3]> {
    let h = s.strip_prefix('#')?;
    if h.len() != 6 || !h.is_ascii() {
        return None;
    }
    let v = u32::from_str_radix(h, 16).ok()?;
    Some([(v >> 16) as u8, (v >> 8) as u8, v as u8])
}

impl ThemeSpec {
    pub fn parse(text: &str, name: &str) -> Result<Self, ConfigError> {
        let f: ThemeFile = toml::from_str(text)
            .map_err(|e| ConfigError::Parse { path: name.into(), message: e.to_string() })?;
        let color = |v: &str| {
            parse_hex(v).ok_or_else(|| ConfigError::BadColor { name: name.into(), value: v.into() })
        };
        if f.ansi.len() != 16 {
            return Err(ConfigError::Parse {
                path: name.into(),
                message: format!("`ansi` needs 16 colours, found {}", f.ansi.len()),
            });
        }
        let mut ansi = [[0; 3]; 16];
        for (dst, src) in ansi.iter_mut().zip(&f.ansi) {
            *dst = color(src)?;
        }
        let foreground = color(&f.foreground)?;
        Ok(Self {
            foreground,
            background: color(&f.background)?,
            cursor: f.cursor.as_deref().map(color).transpose()?.unwrap_or(foreground),
            selection: f.selection.as_deref().map(color).transpose()?.unwrap_or(ansi[4]),
            accent: f.accent.as_deref().map(color).transpose()?.unwrap_or([0x7c, 0x3a, 0xed]),
            ansi,
        })
    }
}

pub const BUILTIN_THEMES: &[(&str, &str)] = &[
    ("z-dark", include_str!("../../../themes/z-dark.toml")),
    ("z-light", include_str!("../../../themes/z-light.toml")),
    ("catppuccin-mocha", include_str!("../../../themes/catppuccin-mocha.toml")),
    ("tokyo-night", include_str!("../../../themes/tokyo-night.toml")),
    ("dracula", include_str!("../../../themes/dracula.toml")),
    ("gruvbox-dark", include_str!("../../../themes/gruvbox-dark.toml")),
    ("nord", include_str!("../../../themes/nord.toml")),
    ("solarized-dark", include_str!("../../../themes/solarized-dark.toml")),
    ("one-dark", include_str!("../../../themes/one-dark.toml")),
    ("rose-pine", include_str!("../../../themes/rose-pine.toml")),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_file_is_default() {
        assert_eq!(Config::parse("", Path::new("x")).unwrap(), Config::default());
    }

    #[test]
    fn partial_config_overrides_only_given_keys() {
        let c = Config::parse("[font]\nsize = 18\n[shell]\nprogram = \"zsh\"", Path::new("x")).unwrap();
        assert_eq!(c.font.size, 18.0);
        assert_eq!(c.font.line_height, 1.25);
        assert_eq!(c.shell.program, "zsh");
        assert_eq!(c.theme, "z-dark");
    }

    #[test]
    fn unknown_key_is_a_readable_error() {
        let e = Config::parse("[font]\nsise = 18", Path::new("/c/zhell.toml")).unwrap_err().to_string();
        assert!(e.contains("/c/zhell.toml") && e.contains("sise"), "{e}");
    }

    #[test]
    fn values_are_clamped() {
        let c = Config::parse("[font]\nsize = 0\n[window]\npadding = -3", Path::new("x")).unwrap();
        assert_eq!(c.font.size, 4.0);
        assert_eq!(c.window.padding, 0.0);
    }

    #[test]
    fn all_builtin_themes_parse() {
        for (name, text) in BUILTIN_THEMES {
            ThemeSpec::parse(text, name).unwrap_or_else(|e| panic!("{name}: {e}"));
        }
        let c = Config { theme: "nope".into(), ..Default::default() };
        assert!(matches!(c.load_theme(None), Err(ConfigError::UnknownTheme(_))));
    }

    #[test]
    fn key_overrides_are_validated() {
        let c = Config::parse("[keys]\n\"alt+t\" = \"new_tab\"", Path::new("x")).unwrap();
        assert_eq!(c.keys["alt+t"], crate::keys::Action::NewTab);
        assert!(Config::parse("[keys]\n\"hyper+t\" = \"new_tab\"", Path::new("x")).is_err());
        assert!(Config::parse("[keys]\n\"alt+t\" = \"fly\"", Path::new("x")).is_err());
    }

    #[test]
    fn editor_template() {
        let l = LinksConfig { editor: "code -g {file}:{line}:{col}".into() };
        assert_eq!(
            l.editor_command("/a b/c.rs", Some(4), None).unwrap(),
            vec!["code", "-g", "/a b/c.rs:4:1"]
        );
        assert!(LinksConfig::default().editor_command("x", None, None).is_none());
    }

    #[test]
    fn short_error_names_line_and_reason() {
        let e = Config::parse("[font]\nsize = \"big\"\n", Path::new("/very/long/path/zhell.toml")).unwrap_err();
        let s = e.short();
        assert!(s.starts_with("zhell.toml line 2: "), "{s}");
        assert!(s.contains("expected f32"), "{s}");
    }

    #[test]
    fn template_is_valid_and_default() {
        assert_eq!(Config::parse(DEFAULT_TEMPLATE, Path::new("t")).unwrap(), Config::default());

        let uncommented: String = DEFAULT_TEMPLATE
            .lines()
            .map(|l| l.strip_prefix("# ").filter(|r| r.contains('=') || r.starts_with('[')).unwrap_or(l))
            .collect::<Vec<_>>()
            .join("\n");
        Config::parse(&uncommented, Path::new("t")).unwrap_or_else(|e| panic!("{e}"));
    }

    #[test]
    fn hex_parsing() {
        assert_eq!(parse_hex("#7c3aed"), Some([0x7c, 0x3a, 0xed]));
        assert_eq!(parse_hex("7c3aed"), None);
        assert_eq!(parse_hex("#7c3ae"), None);
    }
}
