#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod anim;
mod background;
mod copymode;
mod dialog;
mod draw;
mod findbar;
mod glass;
mod input;
mod menu;
mod mirror;
mod palette;
mod remote;
mod quickselect;
mod search;
mod share;
mod theme;
mod titlebar;
mod view;

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, Ime, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::ModifiersState;
use winit::window::{CursorIcon, Window, WindowId};
use zhell_core::SessionHost;
use zhell_core::config::{self, Config, CursorStyle, Decorations};
use zhell_core::keys::{Action, Keymap};
use zhell_core::layout::{Direction, Divider, Layout, Rect, SplitDir};
use zhell_daemon::InProcHost;
use zhell_daemon::ipc::IpcHost;
use zhell_proto::{
    ClientMsg, CopyTarget, CursorShape, HostOptions, PROTO_VERSION, PaneId, Restore, SelectKind,
    ServerMsg, SpawnSpec, TermSize, mode,
};
use zhell_render::{FontConfig, FrameOutcome, Renderer};

use input::MouseButton;
use theme::Theme;
use view::PaneView;

const BLINK_INTERVAL: Duration = Duration::from_millis(530);

const SESSION_ENV: &[&str] = &[
    "DISPLAY",
    "WAYLAND_DISPLAY",
    "XAUTHORITY",
    "SSH_AUTH_SOCK",
    "SSH_AGENT_PID",
    "DBUS_SESSION_BUS_ADDRESS",
    "XDG_SESSION_TYPE",
    "XDG_CURRENT_DESKTOP",
    "XDG_SESSION_DESKTOP",
    "GPG_AGENT_INFO",
];

const REMOTE_PROGRAMS: &[&str] = &["ssh", "mosh", "mosh-client", "telnet", "nc", "ncat", "socat", "irssi", "weechat", "telegram-cli", "gomuks", "ii"];

const SHELLS: &[&str] = &["sh", "bash", "zsh", "fish", "dash", "ksh", "mksh", "tcsh", "csh", "nu", "pwsh", "powershell"];

const MULTI_CLICK: Duration = Duration::from_millis(400);

#[derive(Debug)]
enum UserEvent {
    Wake,

    ConfigChanged,

    BackgroundChanged,

    FontsReady,

    BackgroundLoaded(Box<background::Loaded>),
}

#[derive(Clone, Copy, Debug)]
enum Placement {
    NewTab,
    Split(SplitDir),
}

#[derive(Clone, Debug, Default)]
struct PaneSetup {
    cwd: Option<String>,
    title: Option<String>,

    input: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
enum LinkTarget {
    Url(String),
    Path { path: String, line: Option<u32>, col: Option<u32> },
}

#[derive(Clone, Debug, PartialEq)]
struct HoverLink {
    pane: PaneId,
    cells: Vec<(u16, u16)>,
    target: LinkTarget,
}

fn openable_url(url: &str) -> bool {
    ["http://", "https://", "ftp://", "mailto:"].iter().any(|s| url.starts_with(s))
}

#[derive(Clone, Copy, Debug)]
enum Drag {
    Select { pane: PaneId, moved: bool },
    Divider(Divider),

    Tab(usize),
}

struct Gui {
    window: Arc<Window>,
    renderer: Renderer,
    host: Box<dyn SessionHost>,

    cli_program: Option<(String, Vec<String>)>,

    attach_pane: Option<PaneId>,
    layout: Layout,
    panes: BTreeMap<PaneId, PaneView>,
    pending: HashMap<u32, (Placement, PaneSetup)>,
    next_req: u32,
    theme: Theme,
    keymap: Keymap,
    config: Config,
    config_path: Option<PathBuf>,
    _watcher: Option<notify::RecommendedWatcher>,

    _bg_watcher: Option<notify::RecommendedWatcher>,

    bg_generation: u64,

    bg_frame_at: Instant,

    bg_settle: bool,

    appear: HashMap<&'static str, anim::Spring>,

    toast_shown: String,

    tab_slide: Option<anim::Slide>,

    anim_last: Instant,

    proxy: EventLoopProxy<UserEvent>,
    scale: f32,

    font_size: f32,
    modifiers: ModifiersState,
    focused: bool,
    cursor_pos: PhysicalPosition<f64>,

    mouse_down: Option<MouseButton>,
    drag: Option<Drag>,
    hover_link: Option<HoverLink>,

    title_hover: Option<titlebar::Hit>,
    last_title_click: Option<Instant>,
    last_tab_click: Option<(Instant, usize)>,

    cursor_anim: Option<(PaneId, f32, f32, Instant)>,

    animating: bool,

    saving: Option<String>,

    pending_input: HashMap<PaneId, (String, Instant)>,

    toast: Option<(String, Instant, bool)>,

    offered_projects: std::collections::HashSet<PathBuf>,

    screen_share_manual: Option<bool>,

    recorder_running: bool,
    next_recorder_check: Instant,

    background: u32,

    reconnects: u32,

    closing_keep: Option<bool>,

    glass_state: Option<(bool, u32, u32, u32)>,

    window_close_requested: bool,

    dialog: Option<dialog::Dialog>,

    pending_paste: Option<(u32, String)>,

    pending_install: Option<(u32, PaneId)>,

    copy_mode: Option<copymode::CopyMode>,

    quick: Option<quickselect::QuickSelect>,

    opacity_override: Option<f32>,

    quake: bool,

    quake_had_focus: bool,

    pending_export: Option<(u32, String)>,

    broadcast: Option<PaneId>,

    install_waiting: Option<(PaneId, Instant)>,

    menu: Option<menu::Menu>,

    menu_link: Option<HoverLink>,

    findbar: Option<findbar::FindBar>,

    palette: Option<palette::Palette>,

    search: Option<search::SearchOverlay>,

    hovered_block: Option<(PaneId, u32)>,

    block_buttons: Vec<(PaneId, draw::BlockButton)>,

    pending_paths: HashMap<u32, (Option<u32>, Option<u32>)>,

    last_click: Option<(Instant, PaneId, (u16, u16), u8)>,

    blink_on: bool,
    blink_at: Instant,

    tick_at: Instant,

    preedit: Option<String>,

    ime_area: Option<(i32, i32)>,
    clipboard: Option<arboard::Clipboard>,
}

struct App {
    proxy: EventLoopProxy<UserEvent>,
    gui: Option<Gui>,
    error: Option<anyhow::Error>,
}

impl App {
    fn init(&mut self, el: &ActiveEventLoop) -> anyhow::Result<()> {
        let config_path = config::default_path();
        let mut startup_error = None;
        let config = match config_path.as_deref().map(Config::load) {
            Some(Ok(c)) => c,
            Some(Err(e)) => {
                log::error!("{e}; using defaults");
                startup_error = Some(e.short());
                Config::default()
            }
            None => Config::default(),
        };
        let theme = load_theme(&config, config_path.as_deref(), zhell_core::appearance::prefers_dark());
        let cli = Cli::parse(std::env::args().skip(1));

        let saved = if cli.quake { None } else { WindowState::load() };
        let (w, h) = saved.as_ref().map_or((config.window.width, config.window.height), |s| (s.width, s.height));
        let attrs = Window::default_attributes()
            .with_title("Zhell")
            .with_inner_size(LogicalSize::new(w, h))
            .with_min_inner_size(LogicalSize::new(200.0, 120.0))
            .with_decorations(config.window.decorations == Decorations::System)

            .with_transparent(config.window.decorations == Decorations::Custom || config.window.opacity < 1.0)
            .with_window_icon(titlebar::window_icon());

        #[cfg(all(unix, not(target_os = "macos")))]
        let attrs = {
            let a = winit::platform::x11::WindowAttributesExtX11::with_name(attrs, "zhell", "zhell");
            winit::platform::wayland::WindowAttributesExtWayland::with_name(a, "zhell", "zhell")
        };
        let mut attrs = attrs;
        if cli.quake {
            let monitor = el.primary_monitor().or_else(|| el.available_monitors().next());
            if let Some(m) = &monitor {
                let (pos, size) = quake_geometry(m, &config.quake);
                attrs = attrs.with_position(pos).with_inner_size(size);
            }
            attrs = attrs.with_decorations(false).with_window_level(winit::window::WindowLevel::AlwaysOnTop).with_visible(false);
            #[cfg(windows)]
            {
                attrs = winit::platform::windows::WindowAttributesExtWindows::with_skip_taskbar(attrs, true);
            }
        }
        if let Some(s) = &saved {
            let on_screen = el.available_monitors().any(|m| {
                let (p, sz) = (m.position(), m.size());
                s.x >= p.x && s.y >= p.y && s.x < p.x + sz.width as i32 - 50 && s.y < p.y + sz.height as i32 - 50
            });
            if on_screen {
                attrs = attrs.with_position(winit::dpi::PhysicalPosition::new(s.x, s.y));
            }
            attrs = attrs.with_maximized(s.maximized);
        }
        startup_mark("config");
        let window = Arc::new(el.create_window(attrs)?);
        startup_mark("window");
        window.set_ime_allowed(true);
        if cli.quake {
            glass::set_dropdown_hints(&window);
            window.set_visible(true);
            window.focus_window();
        }
        let scale = window.scale_factor() as f32;
        let size = window.inner_size();
        let font = font_config(&config, config.font.size * scale);
        let renderer = pollster::block_on(Renderer::new(window.clone(), size.width, size.height, font))?;
        startup_mark("gpu+fonts");

        let mut host = start_host(&config)?;
        startup_mark("host");
        let proxy = std::sync::Mutex::new(self.proxy.clone());
        host.set_waker(Box::new(move || {
            let _ = proxy.lock().map(|p| p.send_event(UserEvent::Wake));
        }));

        let mut gui = Gui {
            window,
            renderer,
            host,
            cli_program: None,
            attach_pane: None,
            layout: Layout::default(),
            panes: BTreeMap::new(),
            pending: HashMap::new(),
            next_req: 1,
            theme,
            keymap: Keymap::with_overrides(&config.keys).unwrap_or_default(),
            _watcher: config_path.as_deref().and_then(|p| watch_config(p, self.proxy.clone())),
            _bg_watcher: None,
            bg_generation: 0,
            bg_frame_at: Instant::now(),
            bg_settle: false,
            appear: HashMap::new(),
            toast_shown: String::new(),
            tab_slide: None,
            anim_last: Instant::now(),
            proxy: self.proxy.clone(),
            config_path,
            font_size: config.font.size,
            config,
            scale,
            modifiers: ModifiersState::empty(),
            focused: true,
            cursor_pos: PhysicalPosition::new(0.0, 0.0),
            mouse_down: None,
            drag: None,
            hover_link: None,
            title_hover: None,
            last_title_click: None,
            last_tab_click: None,
            window_close_requested: false,
            glass_state: None,
            closing_keep: None,
            reconnects: 0,
            cursor_anim: None,
            animating: false,
            background: 0,
            screen_share_manual: None,
            toast: None,
            pending_input: HashMap::new(),
            saving: None,
            offered_projects: Default::default(),
            recorder_running: false,
            next_recorder_check: Instant::now(),
            search: None,
            palette: None,
            findbar: None,
            menu: None,
            menu_link: None,
            dialog: None,
            pending_paste: None,
            pending_install: None,
            install_waiting: None,
            broadcast: None,
            pending_export: None,
            quake: false,
            quick: None,
            copy_mode: None,
            opacity_override: None,
            quake_had_focus: false,
            hovered_block: None,
            block_buttons: Vec::new(),
            pending_paths: HashMap::new(),
            last_click: None,
            blink_on: true,
            blink_at: Instant::now() + BLINK_INTERVAL,
            tick_at: Instant::now(),
            preedit: None,
            ime_area: None,
            clipboard: arboard::Clipboard::new().ok(),
        };
        gui.load_background();

        gui.follow_system_theme();

        let restore = !cli.new_window && !cli.quake && cli.attach.is_none() && cli.program.is_none();
        let name = if cli.quake { zhell_daemon::QUAKE_CLIENT } else { "zhell" };
        gui.quake = cli.quake;
        gui.host.send(ClientMsg::Hello { proto_version: PROTO_VERSION, client_name: name.into(), restore });
        gui.host.send(ClientMsg::SetOptions(host_options(&gui.config)));

        if let Some(e) = startup_error {
            gui.show_error(format!("Settings not loaded — {e}"));
        }
        gui.cli_program = cli.program;
        gui.attach_pane = cli.attach;
        self.gui = Some(gui);
        Ok(())
    }
}

fn recorder_running() -> bool {
    #[cfg(target_os = "linux")]
    {
        let Ok(dir) = std::fs::read_dir("/proc") else { return false };
        dir.flatten().any(|e| {
            let name = e.file_name();
            name.to_str().is_some_and(|n| n.bytes().all(|b| b.is_ascii_digit()))
                && std::fs::read_to_string(e.path().join("comm")).is_ok_and(|c| matches!(c.trim(), "obs" | "obs-studio" | "obs64"))
        })
    }
    #[cfg(not(target_os = "linux"))]
    false
}

fn template_names(t: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut rest = t;
    while let Some(start) = rest.find("{{") {
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else { break };
        let name = after[..end].trim();
        if !name.is_empty() && !out.iter().any(|n| n == name) {
            out.push(name.to_owned());
        }
        rest = &after[end + 2..];
    }
    out
}

fn apply_template(t: &str, names: &[String], values: &[String]) -> String {
    let mut out = String::new();
    let mut rest = t;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else {
            out.push_str(&rest[start..]);
            return out;
        };
        let name = after[..end].trim();
        match names.iter().position(|n| n == name).and_then(|i| values.get(i)) {
            Some(v) => out.push_str(v),
            None => out.push_str(&rest[start..start + 2 + end + 2]),
        }
        rest = &after[end + 2..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recording_names() {
        let p = recording_path("cargo build --release");
        let name = p.file_name().unwrap().to_string_lossy();
        assert!(name.starts_with("zhell-20") && name.ends_with("-cargo-build-release.cast"), "{name}");
        assert!(p.parent().unwrap().ends_with("Zhell"));
    }

    #[test]
    fn command_line() {
        let p = |a: &[&str]| Cli::parse(a.iter().map(|s| s.to_string()));
        assert_eq!(p(&[]), Cli::default());
        assert_eq!(p(&["--attach", "7"]).attach, Some(PaneId(7)));
        let c = p(&["--new-window", "htop", "-d", "5"]);
        assert!(c.new_window);
        assert_eq!(c.program, Some(("htop".into(), vec!["-d".into(), "5".into()])));

        assert_eq!(p(&["vim", "--new-window"]).program, Some(("vim".into(), vec!["--new-window".into()])));
        assert_eq!(p(&["--", "--attach"]).program, Some(("--attach".into(), vec![])));
    }

    #[test]
    fn templates() {
        let t = "git checkout {{ branch }} && git push origin {{branch}} --{{flag}}";
        let names = template_names(t);
        assert_eq!(names, vec!["branch", "flag"]);
        let out = apply_template(t, &names, &["main".into(), "force".into()]);
        assert_eq!(out, "git checkout main && git push origin main --force");
        assert_eq!(apply_template("a {{x", &[], &[]), "a {{x");
    }
}

fn reduce_motion() -> bool {
    static REDUCE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *REDUCE.get_or_init(|| {
        if std::env::var_os("ZHELL_REDUCE_MOTION").is_some() {
            return true;
        }
        #[cfg(target_os = "linux")]
        {
            if let Some(cfg) = dirs::config_dir().map(|d| d.join("kdeglobals"))
                && let Ok(text) = std::fs::read_to_string(cfg)
                && text.lines().any(|l| l.trim().replace(' ', "") == "AnimationDurationFactor=0")
            {
                return true;
            }

            if let Ok(out) = std::process::Command::new("gsettings").args(["get", "org.gnome.desktop.interface", "enable-animations"]).output()
                && String::from_utf8_lossy(&out.stdout).trim() == "false"
            {
                return true;
            }
        }
        false
    })
}

fn notification_icon() -> String {
    static ICON: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    ICON.get_or_init(|| {
        let installed = ["/usr/share/icons/hicolor/256x256/apps/zhell.png", "/usr/local/share/icons/hicolor/256x256/apps/zhell.png"];
        if installed.iter().any(|p| std::path::Path::new(p).exists()) {
            return "zhell".into();
        }
        let Some(path) = dirs::state_dir().or_else(dirs::data_local_dir).map(|d| d.join("zhell").join("zhell.png")) else {
            return "zhell".into();
        };

        let icon: &[u8] = include_bytes!("../../../assets/zhell.png");
        if std::fs::read(&path).ok().as_deref() != Some(icon) {
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = std::fs::write(&path, icon);
        }
        path.display().to_string()
    })
    .clone()
}

#[derive(Default, serde::Serialize, serde::Deserialize)]
struct Prefs {
    keep_sessions_on_close: Option<bool>,
}

impl Prefs {
    fn path() -> Option<PathBuf> {
        dirs::state_dir().or_else(dirs::data_local_dir).map(|d| d.join("zhell").join("prefs.toml"))
    }

    fn load() -> Self {
        Self::path().and_then(|p| std::fs::read_to_string(p).ok()).and_then(|t| toml::from_str(&t).ok()).unwrap_or_default()
    }

    fn update(f: impl FnOnce(&mut Self)) {
        let mut p = Self::load();
        f(&mut p);
        if let (Some(path), Ok(text)) = (Self::path(), toml::to_string(&p)) {
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = std::fs::write(path, text);
        }
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct WindowState {
    width: f32,
    height: f32,

    x: i32,
    y: i32,
    maximized: bool,
}

impl WindowState {
    fn path() -> Option<PathBuf> {
        dirs::state_dir().or_else(dirs::data_local_dir).map(|d| d.join("zhell").join("window.toml"))
    }

    fn load() -> Option<Self> {
        let s: Self = toml::from_str(&std::fs::read_to_string(Self::path()?).ok()?).ok()?;
        (s.width >= 100.0 && s.height >= 60.0).then_some(s)
    }

    fn save(window: &Window) {
        let Some(path) = Self::path() else { return };
        let maximized = window.is_maximized();
        let scale = window.scale_factor();
        let size = window.inner_size().to_logical::<f32>(scale);
        let pos = window.outer_position().unwrap_or_default();
        let old = Self::load();

        let (width, height, x, y) = match (&old, maximized) {
            (Some(o), true) => (o.width, o.height, o.x, o.y),
            _ => (size.width, size.height, pos.x, pos.y),
        };
        let state = Self { width, height, x, y, maximized };
        if let (Some(dir), Ok(text)) = (path.parent(), toml::to_string(&state)) {
            let _ = std::fs::create_dir_all(dir);
            let _ = std::fs::write(&path, text);
        }
    }
}

fn start_host(config: &Config) -> anyhow::Result<Box<dyn SessionHost>> {
    let inproc = std::env::var_os("ZHELL_INPROC").is_some() || !config.sessions.daemon;
    if !inproc {
        let exe = std::env::current_exe()?;
        let daemon = exe.with_file_name(if cfg!(windows) { "zhelld.exe" } else { "zhelld" });
        match IpcHost::connect_or_spawn(&daemon) {
            Ok(h) => return Ok(Box::new(h)),
            Err(e) => log::warn!("session daemon unavailable ({e}); shells will end with this window"),
        }
    }
    let history = zhell_daemon::history::HistorySettings::from_config(&config.history);
    Ok(Box::new(InProcHost::start_with(history)?))
}

fn font_config(config: &Config, size_px: f32) -> FontConfig {
    let family = Some(config.font.family.clone()).filter(|f| !f.is_empty());
    FontConfig { family, size_px, line_height: config.font.line_height }
}

fn host_options(config: &Config) -> HostOptions {
    HostOptions {
        scrollback_lines: config.scrollback.lines as u32,
        cursor_shape: match config.cursor.style {
            CursorStyle::Block => CursorShape::Block,
            CursorStyle::Beam => CursorShape::Beam,
            CursorStyle::Underline => CursorShape::Underline,
        },
        cursor_blink: config.cursor.blink,
    }
}

fn load_theme(config: &Config, path: Option<&Path>, dark: bool) -> Theme {
    match config.load_theme_for(path, dark) {
        Ok(spec) => Theme::from_spec(&spec),
        Err(e) => {
            log::error!("{e}; using the default theme");
            Theme::default()
        }
    }
}

fn watch_config(path: &Path, proxy: EventLoopProxy<UserEvent>) -> Option<notify::RecommendedWatcher> {
    watch_file(path, proxy, || UserEvent::ConfigChanged)
}

fn watch_file(path: &Path, proxy: EventLoopProxy<UserEvent>, event: fn() -> UserEvent) -> Option<notify::RecommendedWatcher> {
    use notify::Watcher;
    let dir = path.parent()?.to_owned();
    if !dir.exists() {
        return None;
    }
    let name = path.file_name()?.to_owned();
    let proxy = std::sync::Mutex::new(proxy);
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        let Ok(ev) = res else { return };
        let relevant = !ev.kind.is_access() && ev.paths.iter().any(|p| p.file_name() == Some(&name));
        if relevant {
            let _ = proxy.lock().map(|p| p.send_event(event()));
        }
    })
    .map_err(|e| log::warn!("config watcher: {e}"))
    .ok()?;
    watcher
        .watch(&dir, notify::RecursiveMode::NonRecursive)
        .map_err(|e| log::warn!("config watcher: {e}"))
        .ok()?;
    Some(watcher)
}

impl Gui {
    fn padding(&self) -> f32 {
        (self.config.window.padding * self.scale).round()
    }

    fn gap(&self) -> f32 {
        (2.0 * self.scale).round().max(1.0)
    }

    fn update_glass(&mut self) {
        let size = self.window.inner_size();
        let rounded = self.custom_frame() && !self.window.is_maximized() && !self.quake;
        let radius = if rounded { (theme::RADIUS * self.scale).round() as u32 } else { 0 };
        let enabled = self.config.window.blur && self.opacity() < 1.0 && self.renderer.transparent();
        let key = (enabled, size.width, size.height, radius);
        if self.glass_state != Some(key) {
            self.glass_state = Some(key);
            glass::set_blur(&self.window, enabled, size.width, size.height, radius);
        }
    }

    fn custom_frame(&self) -> bool {
        self.config.window.decorations == Decorations::Custom
    }

    fn show_tab_bar(&self) -> bool {
        self.custom_frame() || self.layout.tabs.len() > 1
    }

    fn tab_bar_height(&self) -> f32 {
        if self.show_tab_bar() { titlebar::height(self.renderer.cell_metrics(), self.scale) } else { 0.0 }
    }

    fn tab_infos(&self) -> Vec<draw::TabInfo> {
        self.layout
            .tabs
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let mut ports: Vec<u16> = t.root.panes().iter().filter_map(|p| self.panes.get(p)).flat_map(|v| v.ports.clone()).collect();
                ports.sort_unstable();
                ports.dedup();
                draw::TabInfo {
                    label: self.panes.get(&t.focus).map(PaneView::label).unwrap_or_default(),
                    active: i == self.layout.active,
                    ports,
                    recording: t.root.panes().iter().any(|p| self.panes.get(p).is_some_and(|v| v.recording.is_some())),
                }
            })
            .collect()
    }

    fn title_layout(&self) -> titlebar::Layout {
        let w = self.window.inner_size().width as f32;
        let buttons = if self.custom_frame() { self.config.window.buttons } else { zhell_core::config::Buttons::None };
        titlebar::layout(w, &self.tab_infos(), self.renderer.cell_metrics(), self.scale, buttons)
    }

    fn title_hit(&self, pos: PhysicalPosition<f64>) -> Option<titlebar::Hit> {
        if !self.show_tab_bar() {
            return None;
        }
        self.title_layout().hit(pos.x as f32, pos.y as f32, self.scale)
    }

    fn resize_edge(&self, pos: PhysicalPosition<f64>) -> Option<winit::window::ResizeDirection> {
        use winit::window::ResizeDirection as D;
        if !self.custom_frame() || self.window.is_maximized() {
            return None;
        }
        let size = self.window.inner_size();
        let e = (5.0 * self.scale) as f64;
        let (l, r) = (pos.x < e, pos.x >= size.width as f64 - e);
        let (t, b) = (pos.y < e, pos.y >= size.height as f64 - e);
        Some(match (l, r, t, b) {
            (true, _, true, _) => D::NorthWest,
            (_, true, true, _) => D::NorthEast,
            (true, _, _, true) => D::SouthWest,
            (_, true, _, true) => D::SouthEast,
            (true, ..) => D::West,
            (_, true, ..) => D::East,
            (_, _, true, _) => D::North,
            (_, _, _, true) => D::South,
            _ => return None,
        })
    }

    fn title_click(&mut self, hit: titlebar::Hit) {
        use titlebar::Hit;
        match hit {
            Hit::Tab(i) => {
                let now = Instant::now();
                let double = self.last_tab_click.is_some_and(|(t, j)| j == i && now - t < MULTI_CLICK);
                self.last_tab_click = Some((now, i));
                if double {
                    self.rename_tab(i);
                } else {
                    self.with_layout(|l, _, _| l.select_tab(i));

                    self.drag = Some(Drag::Tab(i));
                }
            }
            Hit::CloseTab(i) => self.close_tab(i),
            Hit::NewTab => self.spawn(Placement::NewTab, None),
            Hit::Port(tab, port) => {
                if self.modifiers.shift_key() {
                    let pane = self.layout.tabs.get(tab).and_then(|t| {
                        t.root.panes().into_iter().find(|p| self.panes.get(p).is_some_and(|v| v.ports.contains(&port)))
                    });
                    if let Some(pane) = pane {
                        let body = vec![format!("Stop the program listening on port {port}?"), "It gets a polite SIGTERM.".into()];
                        self.dialog = Some(dialog::Dialog::new("Stop server", body, "Stop", dialog::Confirm::StopPort(pane, port)));
                    }
                } else if let Err(e) = open::that_detached(format!("http://localhost:{port}/")) {
                    log::warn!("open port {port}: {e}");
                }
            }
            Hit::Minimize => self.window.set_minimized(true),
            Hit::Maximize => self.window.set_maximized(!self.window.is_maximized()),
            Hit::Close => {
                self.request_close();
            }
            Hit::Drag => {
                let now = Instant::now();
                let double = self.last_title_click.is_some_and(|t| now - t < MULTI_CLICK);
                self.last_title_click = Some(now);
                if double {
                    self.window.set_maximized(!self.window.is_maximized());
                } else if let Err(e) = self.window.drag_window() {
                    log::debug!("drag window: {e}");
                }
            }
        }
    }

    fn close_tab(&mut self, i: usize) {
        if let Some(t) = self.layout.tabs.get(i) {
            for pane in t.root.panes() {
                self.host.send(ClientMsg::ClosePane { pane });
            }
        }
    }

    fn content_area(&self) -> Rect {
        let size = self.window.inner_size();
        let top = self.tab_bar_height();
        Rect { x: 0.0, y: top, w: size.width as f32, h: (size.height as f32 - top).max(1.0) }
    }

    fn grid_for(&self, rect: Rect) -> TermSize {
        let m = self.renderer.cell_metrics();
        let pad = self.padding() * 2.0;
        TermSize {
            cols: (((rect.w - pad) / m.width).floor().max(2.0)) as u16,
            rows: (((rect.h - pad) / m.height).floor().max(1.0)) as u16,
            cell_width: m.width as u16,
            cell_height: m.height as u16,
        }
    }

    fn visible(&self) -> Vec<(PaneId, Rect)> {
        let area = self.content_area();
        self.layout.active_tab().map(|t| t.rects(area, self.gap())).unwrap_or_default()
    }

    fn relayout(&mut self) {
        let area = self.content_area();
        let gap = self.gap();
        let mut sizes = Vec::new();
        for tab in &self.layout.tabs {
            for (id, rect) in tab.rects(area, gap) {
                sizes.push((id, self.grid_for(rect)));
            }

            if tab.zoomed {
                for (id, rect) in tab.root.rects(area, gap) {
                    if id != tab.focus {
                        sizes.push((id, self.grid_for(rect)));
                    }
                }
            }
        }
        for (id, size) in sizes {
            if let Some(p) = self.panes.get_mut(&id)
                && p.grid != size
            {
                p.grid = size;
                self.host.send(ClientMsg::Resize { pane: id, size });
            }
        }
        self.update_title();
        self.store_layout();
        self.window.request_redraw();
    }

    fn pane_at(&self, pos: PhysicalPosition<f64>) -> Option<(PaneId, Rect)> {
        self.visible().into_iter().find(|(_, r)| r.contains(pos.x as f32, pos.y as f32))
    }

    fn divider_at(&self, pos: PhysicalPosition<f64>) -> Option<Divider> {
        let tab = self.layout.active_tab()?;
        if tab.zoomed {
            return None;
        }
        let slop = 3.0 * self.scale;
        let (x, y) = (pos.x as f32, pos.y as f32);
        tab.root.dividers(self.content_area(), self.gap()).into_iter().find(|d| {
            let r = d.rect;
            x >= r.x - slop && x < r.x + r.w + slop && y >= r.y - slop && y < r.y + r.h + slop
        })
    }

    fn cell_in(&self, pane: PaneId, rect: Rect, pos: PhysicalPosition<f64>) -> (i32, u16, bool) {
        let m = self.renderer.cell_metrics();
        let pad = self.padding() as f64;
        let fx = ((pos.x - rect.x as f64 - pad) / m.width as f64).max(0.0);
        let row = ((pos.y - rect.y as f64 - pad) / m.height as f64).floor() as i32;
        let cols = self.panes.get(&pane).map_or(1, |p| p.mirror.cols);
        let col = (fx.floor() as u16).min(cols.saturating_sub(1));
        (row, col, fx.fract() >= 0.5)
    }

    fn link_at(&self, pos: PhysicalPosition<f64>) -> Option<HoverLink> {
        use zhell_proto::flags;
        let (pane, rect) = self.pane_at(pos)?;
        let view = self.panes.get(&pane)?;
        let lines = &view.mirror.lines;
        let (row, col, _) = self.cell_in(pane, rect, pos);
        let (row, col) = (usize::try_from(row).ok()?, col as usize);
        let cell = lines.get(row)?.get(col)?;

        if let Some(uri) = &cell.hyperlink {
            let line = &lines[row];
            let same = |c: usize| line.get(c).is_some_and(|x| x.hyperlink.as_ref() == Some(uri));
            let start = (0..=col).rev().take_while(|&c| same(c)).last().unwrap_or(col);
            let end = (col..line.len()).take_while(|&c| same(c)).last().unwrap_or(col);
            let target = match uri.strip_prefix("file://") {
                Some(rest) => LinkTarget::Path { path: rest[rest.find('/').unwrap_or(0)..].to_owned(), line: None, col: None },
                None if openable_url(uri) => LinkTarget::Url(uri.clone()),
                None => return None,
            };
            let cells = (start..=end).map(|c| (row as u16, c as u16)).collect();
            return Some(HoverLink { pane, cells, target });
        }

        let wraps = |r: usize| lines.get(r).and_then(|l| l.last()).is_some_and(|c| c.flags & flags::WRAPLINE != 0);
        let mut first = row;
        while first > 0 && wraps(first - 1) {
            first -= 1;
        }
        let mut last = row;
        while wraps(last) && last + 1 < lines.len() {
            last += 1;
        }
        let mut text = String::new();
        let mut map = Vec::new();
        let mut pointer = None;
        for (r, line) in lines.iter().enumerate().take(last + 1).skip(first) {
            for (c, cell) in line.iter().enumerate() {
                if cell.flags & (flags::WIDE_CHAR_SPACER | flags::LEADING_WIDE_CHAR_SPACER) != 0 {
                    continue;
                }
                if (r, c) == (row, col) {
                    pointer = Some(map.len());
                }
                text.push(if cell.ch == '\0' { ' ' } else { cell.ch });
                map.push((r as u16, c as u16));
            }
        }
        let m = zhell_core::links::at(&text, pointer?)?;
        let target = match m.kind {
            zhell_core::links::LinkKind::Url if openable_url(&m.target) => LinkTarget::Url(m.target),
            zhell_core::links::LinkKind::Url => return None,
            zhell_core::links::LinkKind::Path { line, col } => LinkTarget::Path { path: m.target, line, col },
        };
        Some(HoverLink { pane, cells: map[m.start..m.end].to_vec(), target })
    }

    fn update_hover(&mut self) {
        let link = if self.modifiers.control_key() && self.drag.is_none() { self.link_at(self.cursor_pos) } else { None };
        if link != self.hover_link {
            self.window.set_cursor(if link.is_some() { CursorIcon::Pointer } else { CursorIcon::Text });
            self.hover_link = link;
            self.window.request_redraw();
        }
    }

    fn open_link(&mut self, link: HoverLink) {
        match link.target {
            LinkTarget::Url(url) => {
                if let Err(e) = open::that_detached(&url) {
                    log::warn!("open {url}: {e}");
                }
            }
            LinkTarget::Path { path, line, col } => {
                let req = self.next_req;
                self.next_req += 1;
                self.pending_paths.insert(req, (line, col));
                self.host.send(ClientMsg::ResolvePath { pane: link.pane, req, path });
            }
        }
    }

    fn open_file(&self, path: &str, line: Option<u32>, col: Option<u32>) {
        let result = match self.config.links.editor_command(path, line, col) {
            Some(cmd) => std::process::Command::new(&cmd[0])
                .args(&cmd[1..])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .map(drop),
            None => open::that_detached(path),
        };
        if let Err(e) = result {
            log::warn!("open {path}: {e}");
        }
    }

    fn rect_of(&self, pane: PaneId) -> Option<Rect> {
        self.visible().into_iter().find(|(p, _)| *p == pane).map(|(_, r)| r)
    }

    fn focused_pane(&self) -> Option<&PaneView> {
        self.layout.focused().and_then(|id| self.panes.get(&id))
    }

    fn spawn(&mut self, placement: Placement, program: Option<(String, Vec<String>)>) {
        self.spawn_with(placement, program, PaneSetup::default());
    }

    fn spawn_with(&mut self, placement: Placement, program: Option<(String, Vec<String>)>, setup: PaneSetup) {
        let shell = &self.config.shell;
        let (program, args) = match program {
            Some((p, a)) => (Some(p), a),
            None if !shell.program.is_empty() => (Some(shell.program.clone()), shell.args.clone()),
            None => (None, Vec::new()),
        };

        let mut env: Vec<(String, String)> = SESSION_ENV
            .iter()
            .filter_map(|k| std::env::var(k).ok().map(|v| ((*k).to_owned(), v)))
            .collect();
        env.extend(shell.env.iter().map(|(k, v)| (k.clone(), v.clone())));

        let cwd = setup.cwd.clone().or_else(|| self.focused_pane().and_then(|p| p.cwd.clone()));

        let size = self.grid_for(self.content_area());
        let req = self.next_req;
        self.next_req += 1;
        self.pending.insert(req, (placement, setup));
        self.host.send(ClientMsg::CreatePane { req, spawn: SpawnSpec { program, args, cwd, env, integration: self.config.shell.integration }, size });
    }

    fn placed(&mut self, req: u32, pane: PaneId) {
        let (placement, setup) = self.pending.remove(&req).unwrap_or((Placement::NewTab, PaneSetup::default()));
        let size = self.grid_for(self.content_area());
        let mut view = PaneView::new(pane, size);
        view.fixed_title = setup.title;
        self.panes.insert(pane, view);
        if let Some(input) = setup.input.filter(|i| !i.is_empty()) {
            self.pending_input.insert(pane, (input, Instant::now() + Duration::from_secs(8)));
        }
        let old_focus = self.layout.focused();
        match placement {
            Placement::Split(dir) if self.layout.split_focused(dir, pane) => {}
            _ => self.layout.add_tab(pane),
        }
        self.focus_changed(old_focus);
        self.relayout();
    }

    fn restore(&mut self, restore: Restore) {
        let (mut layout, titles): (Layout, Vec<(PaneId, String)>) =
            bincode::serde::decode_from_slice(&restore.layout, bincode::config::standard())
                .map(|(l, _)| l)
                .unwrap_or_default();

        for pane in layout.all_panes() {
            if !restore.panes.contains(&pane) {
                layout.remove_pane(pane);
            }
        }
        let known = layout.all_panes();
        for pane in &restore.panes {
            if !known.contains(pane) {
                layout.add_tab(*pane);
            }
        }
        layout.active = layout.active.min(layout.tabs.len().saturating_sub(1));
        log::info!("restoring {} session(s) in {} tab(s)", restore.panes.len(), layout.tabs.len());
        self.layout = layout;
        let size = self.grid_for(self.content_area());
        for pane in restore.panes {
            let mut view = PaneView::new(pane, size);
            view.grid.cols = 0;
            view.fixed_title = titles.iter().find(|(p, _)| *p == pane).map(|(_, t)| t.clone());
            self.panes.insert(pane, view);
            self.host.send(ClientMsg::Attach { pane });
        }
        self.focus_changed(None);
        self.relayout();
    }

    fn adopt(&mut self, pane: PaneId) {
        let old = self.layout.focused();
        self.layout.add_tab(pane);
        let mut view = PaneView::new(pane, self.grid_for(self.content_area()));

        view.grid.cols = 0;
        self.panes.insert(pane, view);
        self.host.send(ClientMsg::Attach { pane });
        self.focus_changed(old);
        self.relayout();
    }

    fn offer_remote_install(&mut self, pane: PaneId, program: Option<&str>) {
        let is_ssh = program.is_some_and(|p| matches!(p, "ssh" | "mosh" | "mosh-client" | "ssh.exe"));
        if !is_ssh {
            self.show_error("Connect with ssh first: this installs on the machine whose prompt is showing".into());
            return;
        }
        let body = vec![
            "Command blocks, history and the directory in the tab then work on this host too.".into(),
            "Types one command at the remote prompt: it copies Zhell's scripts to ~/.local/share/zhell and adds one line to .bashrc / .zshrc (fish: conf.d). Nothing is downloaded.".into(),
            "Make sure the remote shell's prompt is showing.".into(),
        ];
        self.dialog = Some(dialog::Dialog::new("Install shell integration on this host?", body, "Install", dialog::Confirm::InstallRemote(pane)));
        self.window.request_redraw();
    }

    fn copy_mode_effect(&mut self, effect: Option<copymode::Effect>) {
        let Some(pane) = self.copy_mode.as_ref().map(|c| c.pane) else { return };
        match effect {
            Some(copymode::Effect::Send(cmd)) => {
                if matches!(cmd, zhell_proto::CopyCmd::Exit | zhell_proto::CopyCmd::Yank) {
                    self.copy_mode = None;
                    if cmd == zhell_proto::CopyCmd::Yank {
                        self.show_toast("Copied".into());
                    }
                }
                self.host.send(ClientMsg::CopyMode { pane, cmd });
            }
            Some(copymode::Effect::Find) => {
                self.run(Action::Find);
            }
            _ => {}
        }
        self.window.request_redraw();
    }

    fn quick_select_outcome(&mut self, outcome: Option<quickselect::Outcome>) {
        match outcome {
            Some(quickselect::Outcome::Cancel) => self.quick = None,
            Some(quickselect::Outcome::Pick { text, paste }) => {
                self.quick = None;
                self.set_clipboard(text.clone(), CopyTarget::Clipboard);
                if paste {
                    self.paste_text(text);
                } else {
                    self.show_toast(format!("Copied {text}"));
                }
            }
            _ => {}
        }
        self.window.request_redraw();
    }

    fn toggle_quake(&mut self) {
        let visible = self.window.is_visible().unwrap_or(true);
        if visible && self.focused {
            self.quake_had_focus = false;
            self.window.set_visible(false);
            return;
        }
        if let Some(m) = self.window.current_monitor().or_else(|| self.window.primary_monitor()) {
            let (pos, size) = quake_geometry(&m, &self.config.quake);
            self.window.set_outer_position(pos);
            let _ = self.window.request_inner_size(size);
        }
        self.quake_had_focus = false;
        self.window.set_visible(true);
        glass::set_dropdown_hints(&self.window);
        self.window.focus_window();
        self.window.request_redraw();
    }

    fn open_window(&mut self, args: &[String]) {
        let child = std::env::current_exe().and_then(|exe| {
            std::process::Command::new(exe)
                .args(args)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::inherit())
                .spawn()
        });
        match child {
            Ok(mut c) => drop(std::thread::Builder::new().name("window".into()).spawn(move || c.wait())),
            Err(e) => self.show_error(format!("Couldn't open a new window: {e}")),
        }
    }

    fn store_layout(&self) {
        let titles: Vec<(PaneId, String)> =
            self.panes.values().filter_map(|v| v.fixed_title.clone().map(|t| (v.id, t))).collect();
        if let Ok(blob) = bincode::serde::encode_to_vec((&self.layout, titles), bincode::config::standard()) {
            self.host.send(ClientMsg::StoreLayout(blob));
        }
    }

    fn request_close(&mut self) -> bool {
        if self.quake {
            self.window.set_visible(false);
            return false;
        }
        let daemon = !self.config.sessions.daemon || !self.config.sessions.keep_alive;
        let sessions = self.layout.all_panes().len();
        if daemon || sessions == 0 || Prefs::load().keep_sessions_on_close.is_some() {
            let keep = Prefs::load().keep_sessions_on_close.unwrap_or(true) && self.config.sessions.keep_alive;
            self.close_now(keep);
            return true;
        }
        let n = if sessions == 1 { "1 session".to_owned() } else { format!("{sessions} sessions") };
        let body = vec![
            format!("Keep {n} running in the background? Reopen Zhell to get them back."),
            "Zhell remembers your answer; change it later under [sessions] in the settings.".into(),
        ];
        let mut d = dialog::Dialog::new("Close window", body, "Keep running", dialog::Confirm::CloseWindow(true))
            .with_alt("End them", dialog::Confirm::CloseWindow(false))
            .default_confirm();
        d.cancel_label = "Don't close".into();
        self.dialog = Some(d);
        self.window.request_redraw();
        false
    }

    fn close_now(&mut self, keep: bool) {
        self.closing_keep = Some(keep);
        self.window_close_requested = true;
    }

    fn closing(&mut self) {
        if !self.quake {
            WindowState::save(&self.window);
        }
        if self.closing_keep == Some(false) {
            for pane in self.layout.all_panes() {
                self.host.send(ClientMsg::ClosePane { pane });
            }
            return;
        }
        if !self.config.sessions.keep_alive {
            for pane in self.layout.all_panes() {
                self.host.send(ClientMsg::ClosePane { pane });
            }
        }
    }

    fn close_focused(&mut self) {
        if let Some(pane) = self.layout.focused() {
            self.host.send(ClientMsg::ClosePane { pane });
        }
    }

    fn pane_exited(&mut self, pane: PaneId) -> bool {
        self.panes.remove(&pane);
        self.copy_mode.take_if(|c| c.pane == pane);
        self.quick.take_if(|q| q.pane == pane);
        let old_focus = self.layout.focused();
        if self.layout.remove_pane(pane) {
            return false;
        }
        self.focus_changed(old_focus.filter(|p| *p != pane));
        self.relayout();
        true
    }

    fn focus_changed(&mut self, old: Option<PaneId>) {
        let new = self.layout.focused();

        if let Some(c) = self.copy_mode.take_if(|c| Some(c.pane) != new) {
            self.host.send(ClientMsg::CopyMode { pane: c.pane, cmd: zhell_proto::CopyCmd::Exit });
        }
        self.quick.take_if(|q| Some(q.pane) != new);
        if old == new {
            return;
        }
        if let Some(pane) = old {
            self.host.send(ClientMsg::Focus { pane, focused: false });
        }
        if let Some(pane) = new {
            self.host.send(ClientMsg::Focus { pane, focused: self.focused });
        }
        self.reset_blink();
        self.update_title();
        self.window.request_redraw();
    }

    fn with_layout(&mut self, f: impl FnOnce(&mut Layout, Rect, f32)) {
        let old = self.layout.focused();
        let area = self.content_area();
        let gap = self.gap();
        f(&mut self.layout, area, gap);
        self.focus_changed(old);

        self.relayout();
    }

    fn update_title(&self) {
        let title = self.focused_pane().map(PaneView::label).unwrap_or_else(|| "Zhell".into());
        self.window.set_title(&title);
    }

    fn reload_config(&mut self) {
        let Some(path) = self.config_path.clone() else { return };
        let new = match Config::load(&path) {
            Ok(c) => c,
            Err(e) => {
                log::error!("{e}; keeping the previous config");
                self.show_error(format!("Settings not applied — {}", e.short()));
                return;
            }
        };
        self.toast = self.toast.take().filter(|t| !t.2);
        if new == self.config {
            return;
        }
        log::info!("config reloaded");
        let font_changed = new.font != self.config.font;
        let background_changed = new.background != self.config.background || new.effects != self.config.effects;
        let options_changed = host_options(&new) != host_options(&self.config);
        self.theme = load_theme(&new, Some(&path), self.system_dark());
        self.keymap = Keymap::with_overrides(&new.keys).unwrap_or_default();
        self.config = new;
        if font_changed {
            self.font_size = self.config.font.size;
            self.renderer.set_font(font_config(&self.config, self.font_size * self.scale));
        }
        if options_changed {
            self.host.send(ClientMsg::SetOptions(host_options(&self.config)));
        }
        if background_changed {
            self.load_background();
        }
        self.relayout();
    }

    fn reload_background(&mut self) {
        self.bg_settle = true;
        self.load_background();
        self.bg_settle = false;
    }

    fn load_background(&mut self) {
        self.bg_generation += 1;
        self.renderer.set_crt(self.config.effects.crt);
        let cfg = self.config.background.clone();
        let dir = self.config_path.as_deref().and_then(Path::parent).map(Path::to_path_buf);
        self._bg_watcher = background::shader_file(&cfg, dir.as_deref())
            .and_then(|f| watch_file(&f, self.proxy.clone(), || UserEvent::BackgroundChanged));
        if !cfg.is_set() {
            let _ = self.renderer.set_background(None);
            self.window.request_redraw();
            return;
        }
        let (generation, proxy) = (self.bg_generation, self.proxy.clone());

        let max_side = self.renderer.max_texture_size().min(4096);
        let settle = self.bg_settle;
        let spawned = std::thread::Builder::new().name("background".into()).spawn(move || {
            if settle {
                std::thread::sleep(Duration::from_millis(150));
            }
            let loaded = background::load(&cfg, dir.as_deref(), max_side, generation);
            let _ = proxy.send_event(UserEvent::BackgroundLoaded(Box::new(loaded)));
        });
        if let Err(e) = spawned {
            log::error!("background loader: {e}");
        }
    }

    fn background_loaded(&mut self, l: background::Loaded) {
        if l.generation != self.bg_generation {
            return;
        }
        let cfg = &self.config.background;
        let bg = (l.image.is_some() || l.shader.is_some()).then(|| zhell_render::Background {
            image: l.image.as_ref().map(|(w, h, px)| (*w, *h, px.as_slice())),
            fit: background::fit(cfg.fit),
            shader: l.shader.as_deref(),
            dim: cfg.dim(),
        });
        let mut errors = l.errors;
        if let Err(e) = self.renderer.set_background(bg) {
            let name = Path::new(cfg.shader.trim()).file_name().map_or_else(|| cfg.shader.clone(), |n| n.to_string_lossy().into_owned());
            log::error!("background shader: {e}");
            errors.push(format!("Shader not applied — {}", background::short_shader_error(&e, &name)));

            if l.shader.is_some() && l.image.is_some() {
                let image = l.image.as_ref().map(|(w, h, px)| (*w, *h, px.as_slice()));
                let _ = self.renderer.set_background(Some(zhell_render::Background { image, fit: background::fit(cfg.fit), shader: None, dim: cfg.dim() }));
            }
        }
        match errors.into_iter().next() {
            Some(e) => self.show_error(e),

            None if self.toast.as_ref().is_some_and(|t| t.2 && (t.0.starts_with("Shader not applied") || t.0.starts_with("Background "))) => self.toast = None,
            None => {}
        }
        self.window.request_redraw();
    }

    fn background_animating(&self) -> bool {
        self.renderer.background_animated()
            && self.config.background.animate
            && self.focused
            && self.window.is_minimized() != Some(true)
            && !reduce_motion()
    }

    fn set_font_size(&mut self, logical: f32) {
        self.font_size = logical.clamp(4.0, 200.0);
        self.renderer.set_font(font_config(&self.config, self.font_size * self.scale));
        self.relayout();
    }

    fn cursor_blinks(&self) -> bool {
        self.focused
            && self.focused_pane().is_some_and(|p| p.mirror.cursor.blinking && p.mirror.cursor.shape != CursorShape::Hidden)
    }

    fn reset_blink(&mut self) {
        self.blink_on = true;
        self.blink_at = Instant::now() + BLINK_INTERVAL;
    }

    fn write(&self, bytes: Vec<u8>) {
        let Some(focused) = self.layout.focused() else { return };
        let targets = self.broadcast_targets().unwrap_or_else(|| vec![focused]);
        for pane in targets {
            let Some(p) = self.panes.get(&pane) else { continue };

            if p.mirror.display_offset > 0 {
                self.host.send(ClientMsg::ScrollToBottom { pane });
            }
            self.host.send(ClientMsg::Input { pane, bytes: bytes.clone() });
        }
    }

    fn broadcast_targets(&self) -> Option<Vec<PaneId>> {
        let anchor = self.broadcast?;
        let panes = self.layout.active_tab()?.root.panes();
        (panes.contains(&anchor) && panes.len() > 1).then_some(panes)
    }

    fn paste(&mut self) {
        let Some(text) = self.clipboard.as_mut().and_then(|c| c.get_text().ok()) else { return };
        self.paste_text(text);
    }

    fn paste_text(&mut self, text: String) {
        let multiline = text.trim_end_matches(['\n', '\r']).contains(['\n', '\r']);
        if self.config.paste.guard
            && (multiline || !zhell_secrets::find(&text).is_empty())
            && let Some(pane) = self.layout.focused()
        {
            let req = self.next_req;
            self.next_req += 1;
            self.pending_paste = Some((req, text));
            self.host.send(ClientMsg::QueryForeground { req, pane });
            return;
        }
        self.paste_checked(text, None);
    }

    fn paste_checked(&mut self, text: String, foreground: Option<&str>) {
        if self.config.paste.guard {
            let mut reasons = Vec::new();
            let remote = foreground.is_some_and(|f| REMOTE_PROGRAMS.contains(&f));
            if remote && let Some(m) = zhell_secrets::find(&text).first() {
                reasons.push(format!("It looks like a secret ({}) and {} will send it", m.kind.label(), foreground.unwrap_or("")));
                reasons.push("to another machine.".into());
            }
            let bracketed = self.focused_pane().is_some_and(|p| p.mirror.modes & mode::BRACKETED_PASTE != 0);
            let lines = text.trim_end_matches(['\n', '\r']).lines().count();
            let runs_lines = foreground.is_some_and(|f| SHELLS.contains(&f) || REMOTE_PROGRAMS.contains(&f));
            if lines > 1 && !bracketed && runs_lines && self.config.paste.warn_multiline {
                reasons.push(format!("It has {lines} lines; the shell may run each line right away."));
            }
            if !reasons.is_empty() {
                let preview: String = text.lines().next().unwrap_or("").chars().take(60).collect();
                let mut body = reasons;
                body.push(String::new());
                body.push(format!("“{preview}{}”", if text.chars().count() > 60 || lines > 1 { "…" } else { "" }));
                self.dialog = Some(dialog::Dialog::new("Paste this?", body, "Paste", dialog::Confirm::Paste(text)));
                self.window.request_redraw();
                return;
            }
        }
        self.paste_now(text);
    }

    fn paste_now(&mut self, text: String) {
        let Some(p) = self.focused_pane() else { return };

        let text = text.replace('\x1b', "").replace("\r\n", "\r").replace('\n', "\r");
        let bytes = if p.mirror.modes & mode::BRACKETED_PASTE != 0 {
            format!("\x1b[200~{text}\x1b[201~").into_bytes()
        } else {
            text.into_bytes()
        };
        self.write(bytes);
    }

    fn paste_primary(&mut self) {
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            use arboard::{GetExtLinux, LinuxClipboardKind};
            let text = self
                .clipboard
                .as_mut()
                .and_then(|c| c.get().clipboard(LinuxClipboardKind::Primary).text().ok());
            if let Some(text) = text {
                self.paste_text(text);
            }
        }
        #[cfg(not(all(unix, not(target_os = "macos"))))]
        self.paste();
    }

    fn copy(&mut self) {
        if let Some(pane) = self.layout.focused() {
            self.host.send(ClientMsg::Copy { pane, target: CopyTarget::Clipboard });
        }
    }

    fn save_output(&mut self, text: String) {
        let cmd = self.saving.take().unwrap_or_else(|| "output".into());
        let path = export_path(&cmd, "txt");
        match std::fs::write(&path, format!("$ {cmd}\n{text}\n")) {
            Ok(()) => self.show_toast(format!("Saved output to {}", path.display())),
            Err(e) => self.show_error(format!("Couldn't save the output: {e}")),
        }
    }

    fn save_html(&mut self, cmd: &str, rows: &[Vec<zhell_proto::Cell>]) {
        if rows.is_empty() {
            self.show_error("That command's output is no longer in the scrollback".into());
            return;
        }
        let path = export_path(cmd, "html");
        let page = share::html(rows, &self.theme, cmd.lines().next().unwrap_or(cmd), &self.config.font.family);
        match std::fs::write(&path, page) {
            Ok(()) => self.show_toast(format!("Saved as a web page: {}", path.display())),
            Err(e) => self.show_error(format!("Couldn't save the page: {e}")),
        }
    }

    fn set_clipboard(&mut self, text: String, target: CopyTarget) {
        if target == CopyTarget::File {
            self.save_output(text);
            return;
        }
        let Some(c) = self.clipboard.as_mut() else { return };
        let res = match target {
            CopyTarget::Clipboard => c.set_text(text),
            #[cfg(all(unix, not(target_os = "macos")))]
            CopyTarget::Primary => {
                use arboard::{LinuxClipboardKind, SetExtLinux};
                c.set().clipboard(LinuxClipboardKind::Primary).text(text)
            }
            #[cfg(not(all(unix, not(target_os = "macos"))))]
            CopyTarget::Primary => Ok(()),
            CopyTarget::File => Ok(()),
        };
        if let Err(e) = res {
            log::warn!("clipboard: {e}");
        }
    }

    fn report_mouse(&mut self, button: Option<MouseButton>, pressed: bool, motion: bool) -> bool {
        if self.modifiers.shift_key() {
            return false;
        }
        let Some((pane, rect)) = self.pane_at(self.cursor_pos) else { return false };
        let Some(view) = self.panes.get(&pane) else { return false };
        let (row, col, _) = self.cell_in(pane, rect, self.cursor_pos);
        let row = row.clamp(0, view.mirror.rows.saturating_sub(1) as i32) as u16;
        match input::encode_mouse(button, pressed, motion, col, row, self.modifiers, view.mirror.modes) {
            Some(bytes) => {
                self.host.send(ClientMsg::Input { pane, bytes });
                true
            }
            None => false,
        }
    }

    fn update_block_hover(&mut self) {
        let hovered = self.pane_at(self.cursor_pos).and_then(|(pane, rect)| {
            let view = self.panes.get(&pane)?;
            let (row, _, _) = self.cell_in(pane, rect, self.cursor_pos);
            let i = view.mirror.block_at(row)?;
            Some((pane, view.mirror.blocks[i].id))
        });
        if hovered != self.hovered_block {
            self.hovered_block = hovered;
            self.window.request_redraw();
        }
    }

    fn find_effect(&mut self, e: findbar::FindEffect) {
        let Some(fb) = &self.findbar else { return };
        let pane = fb.pane;
        match e {
            findbar::FindEffect::None => {}
            findbar::FindEffect::Search => {
                self.host.send(ClientMsg::Find { pane, query: fb.query.clone(), regex: fb.regex });
            }
            findbar::FindEffect::Next { older } => self.host.send(ClientMsg::FindNext { pane, older }),
            findbar::FindEffect::Close => {
                self.findbar = None;
                self.host.send(ClientMsg::FindClose { pane });
            }
        }
        self.window.request_redraw();
    }

    fn reconnect(&mut self) -> bool {
        if self.reconnects >= 3 {
            log::error!("session host keeps failing; giving up");
            return false;
        }
        self.reconnects += 1;
        let mut host = match start_host(&self.config) {
            Ok(h) => h,
            Err(e) => {
                log::error!("restarting the session host: {e}");
                return false;
            }
        };
        let proxy = std::sync::Mutex::new(self.proxy.clone());
        host.set_waker(Box::new(move || {
            let _ = proxy.lock().map(|p| p.send_event(UserEvent::Wake));
        }));
        self.host = host;

        self.panes.clear();
        self.layout = Layout::default();
        self.pending.clear();
        self.pending_input.clear();
        self.hover_link = None;
        self.hovered_block = None;
        self.findbar = None;
        self.host.send(ClientMsg::Hello { proto_version: PROTO_VERSION, client_name: "zhell".into(), restore: true });
        self.host.send(ClientMsg::SetOptions(host_options(&self.config)));
        self.show_error("The session host stopped unexpectedly. Restored your tabs from the last snapshot; running programs were lost.".into());
        true
    }

    fn refresh_search(&mut self) {
        if let Some(q) = self.search.as_ref().map(search::SearchOverlay::query) {
            self.search_effect(search::SearchEffect::Query(q));
        }
    }

    fn fill_template(&mut self, template: String, names: Vec<String>, values: Vec<String>) {
        if let Some(next) = names.get(values.len()) {
            let body = vec![template.clone()];
            let title = format!("{next}  ({} of {})", values.len() + 1, names.len());
            let confirm = dialog::Confirm::FillTemplate { template: template.clone(), names: names.clone(), values };
            self.dialog = Some(dialog::Dialog::new(title, body, "Next", confirm).with_input(String::new()));
            return;
        }
        let cmd = apply_template(&template, &names, &values);

        self.write(cmd.replace('\n', " ").into_bytes());
    }

    fn rename_tab(&mut self, tab: usize) {
        let Some(pane) = self.layout.tabs.get(tab).map(|t| t.focus) else { return };
        let current = self.panes.get(&pane).map(PaneView::label).unwrap_or_default();
        let body = vec!["Leave empty to go back to the automatic name.".into()];
        self.dialog = Some(dialog::Dialog::new("Rename tab", body, "Rename", dialog::Confirm::RenameTab(pane)).with_input(current));
        self.window.request_redraw();
    }

    fn open_menu(&mut self) {
        use menu::{Item, MenuAction as A};
        let key = |a: Action| self.keymap.key_for(a).map(|k| k.to_string()).unwrap_or_default();
        let mut items = Vec::new();
        self.menu_link = self.link_at(self.cursor_pos);
        if let Some(link) = &self.menu_link {
            let what = match &link.target {
                LinkTarget::Url(_) => "Open link",
                LinkTarget::Path { .. } => "Open file",
            };
            items.push(Item::new(what, "ctrl+click".into(), A::OpenLink));
            items.push(Item::separator());
        }
        let has_selection = self.focused_pane().is_some_and(|p| p.mirror.selection.is_some());
        if has_selection {
            items.push(Item::new("Copy", key(Action::Copy), A::Copy));
        }
        items.push(Item::new("Paste", key(Action::Paste), A::Paste));

        if let Some((pane, rect)) = self.pane_at(self.cursor_pos)
            && let Some(view) = self.panes.get(&pane)
            && let Some(i) = view.mirror.block_at(self.cell_in(pane, rect, self.cursor_pos).0)
            && view.mirror.blocks[i].state != zhell_proto::BlockState::Editing
        {
            let b = &view.mirror.blocks[i];
            items.push(Item::separator());
            let folded = view.mirror.folds.iter().any(|f| f.block == b.id);
            items.push(Item::new(if folded { "Expand output" } else { "Collapse output" }, String::new(), A::ToggleFold(pane, b.id)));
            items.push(Item::new("Copy command output", String::new(), A::CopyOutput(pane, b.id)));
            items.push(Item::new("Save command output…", String::new(), A::SaveOutput(pane, b.id, b.cmd.clone())));
            items.push(Item::new("Share as web page…", String::new(), A::ExportHtml(pane, b.id, b.cmd.clone())));
        }
        items.push(Item::separator());
        items.push(Item::new("Split right", key(Action::SplitRight), A::SplitRight));
        items.push(Item::new("Split down", key(Action::SplitDown), A::SplitDown));
        items.push(Item::new("New tab", key(Action::NewTab), A::NewTab));
        items.push(Item::new("New window", key(Action::NewWindow), A::Run(Action::NewWindow)));
        if self.host.shared() {
            items.push(Item::new("Move to new window", key(Action::MoveToNewWindow), A::Run(Action::MoveToNewWindow)));
        }
        items.push(Item::separator());
        items.push(Item::new("Find…", key(Action::Find), A::Find));
        items.push(Item::new("Search history…", key(Action::SearchHistory), A::SearchHistory));
        items.push(Item::new("Command palette…", key(Action::CommandPalette), A::Palette));
        items.push(Item::separator());
        items.push(Item::new("Close pane", key(Action::ClosePane), A::ClosePane));
        self.menu = Some(menu::Menu::new(items, self.cursor_pos.x as f32, self.cursor_pos.y as f32));
        self.window.set_cursor(CursorIcon::Default);
        self.window.request_redraw();
    }

    fn menu_result(&mut self, res: menu::MenuResult) {
        use menu::MenuAction as A;
        match res {
            menu::MenuResult::Open => {}
            menu::MenuResult::Close => self.menu = None,
            menu::MenuResult::Run(action) => {
                self.menu = None;
                match action {
                    A::Copy => self.copy(),
                    A::Paste => self.paste(),
                    A::OpenLink => {
                        if let Some(link) = self.menu_link.take() {
                            self.open_link(link);
                        }
                    }
                    A::CopyOutput(pane, block) => self.block_action(pane, block, draw::BlockAction::CopyOutput),
                    A::ToggleFold(pane, block) => self.block_action(pane, block, draw::BlockAction::ToggleFold),
                    A::SaveOutput(pane, block, _) => self.block_action(pane, block, draw::BlockAction::SaveOutput),
                    A::ExportHtml(pane, block, cmd) => {
                        let req = self.next_req;
                        self.next_req += 1;
                        self.pending_export = Some((req, cmd.unwrap_or_else(|| "output".into())));
                        self.host.send(ClientMsg::BlockCells { req, pane, block });
                    }
                    A::SplitRight => {
                        self.run(Action::SplitRight);
                    }
                    A::SplitDown => {
                        self.run(Action::SplitDown);
                    }
                    A::NewTab => {
                        self.run(Action::NewTab);
                    }
                    A::Find => {
                        self.run(Action::Find);
                    }
                    A::SearchHistory => {
                        self.run(Action::SearchHistory);
                    }
                    A::Palette => {
                        self.run(Action::CommandPalette);
                    }
                    A::ClosePane => {
                        self.run(Action::ClosePane);
                    }
                    A::Run(a) => {
                        self.run(a);
                    }
                }
            }
        }
        self.window.request_redraw();
    }

    fn dialog_result(&mut self, res: dialog::DialogResult) {
        match res {
            dialog::DialogResult::Open => {}
            dialog::DialogResult::Cancel => self.dialog = None,
            dialog::DialogResult::Confirm(c) => {
                let name = self.dialog.take().and_then(|d| d.input).unwrap_or_default();
                match c {
                    dialog::Confirm::Paste(text) => self.paste_now(text),
                    dialog::Confirm::StopPort(pane, port) => self.host.send(ClientMsg::StopPort { pane, port }),
                    dialog::Confirm::OpenUrl(url) => {
                        if let Err(e) = open::that_detached(&url) {
                            self.show_error(format!("Couldn't open {url}: {e}"));
                        }
                    }
                    dialog::Confirm::InstallRemote(pane) => {
                        self.host.send(ClientMsg::Input { pane, bytes: remote::installer_command().into_bytes() });
                        self.install_waiting = Some((pane, Instant::now() + Duration::from_secs(30)));
                    }
                    dialog::Confirm::SetNote(id) => {
                        let note = Some(name.trim().to_owned()).filter(|n| !n.is_empty());
                        self.host.send(ClientMsg::HistoryNote { id, note });
                        self.refresh_search();
                    }
                    dialog::Confirm::SetTemplate(id) => {
                        let template = Some(name.trim().to_owned()).filter(|n| !n.is_empty());
                        if template.is_some() {
                            self.host.send(ClientMsg::HistoryStar { id, starred: true });
                        }
                        self.host.send(ClientMsg::HistoryTemplate { id, template });
                        self.refresh_search();
                    }
                    dialog::Confirm::FillTemplate { template, names, mut values } => {
                        values.push(name);
                        self.fill_template(template, names, values);
                    }
                    dialog::Confirm::CloseWindow(keep) => {
                        Prefs::update(|p| p.keep_sessions_on_close = Some(keep));
                        self.close_now(keep);
                    }
                    dialog::Confirm::RenameTab(pane) => {
                        if let Some(v) = self.panes.get_mut(&pane) {
                            v.fixed_title = Some(name.trim().to_owned()).filter(|n| !n.is_empty());
                        }
                        self.store_layout();
                    }
                }
            }
        }
        self.window.request_redraw();
    }

    fn current_project(&self) -> Option<zhell_core::project::Project> {
        let cwd = self.focused_pane()?.cwd.clone()?;
        let mut dir = std::path::PathBuf::from(cwd);
        loop {
            if let Some(p) = zhell_core::project::detect(&dir) {
                return Some(p);
            }

            if Some(&dir) == dirs::home_dir().as_ref() || !dir.pop() {
                return None;
            }
        }
    }

    fn open_workspace(&mut self, project: &zhell_core::project::Project) {
        for tab in project.workspace().tabs {
            let cwd = if tab.cwd.is_empty() { project.root.clone() } else { project.root.join(&tab.cwd) };
            let setup = PaneSetup {
                cwd: Some(cwd.display().to_string()),
                title: Some(tab.title),
                input: Some(tab.command).filter(|c| !c.is_empty()),
            };
            self.spawn_with(Placement::NewTab, None, setup);
        }
    }

    fn show_toast(&mut self, text: String) {
        self.toast = Some((text, Instant::now() + Duration::from_secs(7), false));
        self.window.request_redraw();
    }

    fn show_error(&mut self, text: String) {
        let line = text.lines().map(str::trim).filter(|l| !l.is_empty() && *l != "|").collect::<Vec<_>>().join(" ");
        self.toast = Some((line, Instant::now() + Duration::from_secs(15), true));
        self.window.request_redraw();
    }

    fn open_settings(&mut self) {
        let Some(path) = self.config_path.clone() else { return };
        if !path.exists() {
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if let Err(e) = std::fs::write(&path, config::DEFAULT_TEMPLATE) {
                self.show_error(format!("Couldn't create {}: {e}", path.display()));
                return;
            }

            if self._watcher.is_none() {
                self._watcher = watch_config(&path, self.proxy.clone());
            }
        }
        self.open_file(&path.display().to_string(), None, None);
        self.show_toast(format!("Editing {} — changes apply when you save", path.display()));
    }

    fn cwd_changed(&mut self) {
        let Some(project) = self.current_project() else { return };
        if !self.offered_projects.insert(project.root.clone()) {
            return;
        }
        let key = self.keymap.key_for(Action::OpenWorkspace).map(|k| k.to_string()).unwrap_or_else(|| "the palette".into());
        let kinds = project.kinds.join(" · ");
        let name = project.root.file_name().and_then(|n| n.to_str()).unwrap_or("project");
        self.show_toast(format!("◆ {name} — {kinds} project · {key} opens its workspace"));
    }

    fn notify_finished(&mut self, f: &zhell_proto::FrameDiff) {
        use zhell_proto::BlockState;
        let Some(view) = self.panes.get(&f.pane) else { return };
        let was_running: Vec<u32> = view.mirror.blocks.iter().filter(|b| b.state == BlockState::Running).map(|b| b.id).collect();
        if was_running.is_empty() || !self.config.notify.enabled {
            return;
        }
        let visible = self.focused && self.visible().iter().any(|(p, _)| *p == f.pane);
        if visible {
            return;
        }
        for b in f.blocks.iter().filter(|b| was_running.contains(&b.id)) {
            let BlockState::Done { exit } = b.state else { continue };
            let ms = b.duration_ms.unwrap_or(0);
            if ms < self.config.notify.min_seconds as u64 * 1000 {
                continue;
            }
            let cmd = b.cmd.clone().unwrap_or_else(|| "command".into());
            let took = if ms >= 60_000 { format!("{} m {} s", ms / 60_000, (ms / 1000) % 60) } else { format!("{:.1} s", ms as f64 / 1000.0) };
            let (summary, body) = match exit {
                Some(0) | None => (format!("✓ {cmd}"), format!("finished in {took}")),
                Some(code) => (format!("✗ {cmd}"), format!("failed with exit code {code} after {took}")),
            };
            let icon = notification_icon();
            std::thread::spawn(move || {
                let _ = notify_rust::Notification::new().appname("Zhell").icon(&icon).summary(&summary).body(&body).show();
            });
        }
    }

    fn screen_sharing(&self) -> bool {
        self.screen_share_manual.unwrap_or(self.recorder_running && self.config.paste.auto_screen_share)
    }

    fn check_recorder(&mut self) {
        if !self.config.paste.auto_screen_share || Instant::now() < self.next_recorder_check {
            return;
        }
        self.next_recorder_check = Instant::now() + Duration::from_secs(4);
        let running = recorder_running();
        if running != self.recorder_running {
            self.recorder_running = running;
            log::info!("screen recorder {}", if running { "detected: hiding secrets" } else { "stopped" });
            self.window.request_redraw();
        }
    }

    fn open_palette(&mut self) {
        use palette::{Item, ItemKind};
        let mut items: Vec<Item> = Action::PALETTE
            .iter()
            .map(|(a, name)| Item {
                title: (*name).to_owned(),
                hint: self.keymap.key_for(*a).map(|k| k.to_string()).unwrap_or_default(),
                kind: ItemKind::Action(*a),
            })
            .collect();
        let user = zhell_core::themes::user_themes();
        for name in config::BUILTIN_THEMES.iter().map(|(n, _)| (*n).to_owned()).chain(user) {
            let current = name == self.config.theme;
            items.push(Item {
                title: format!("Theme: {name}"),
                hint: if current { "current theme".into() } else { "theme".into() },
                kind: ItemKind::Theme(name),
            });
        }
        if let Some(project) = self.current_project() {
            for t in &project.tasks {
                items.push(Item { title: format!("Run: {}", t.name), hint: t.command.clone(), kind: ItemKind::Command(t.command.clone()) });
            }
            items.push(Item {
                title: "Project: save workspace".into(),
                hint: zhell_core::project::WORKSPACE_FILE.into(),
                kind: ItemKind::SaveWorkspace,
            });
        }

        for prof in &self.config.profile {
            let cwd = (!prof.cwd.is_empty()).then(|| match (prof.cwd.strip_prefix("~/"), dirs::home_dir()) {
                (Some(rest), Some(h)) => h.join(rest).display().to_string(),
                _ if prof.cwd == "~" => dirs::home_dir().map(|h| h.display().to_string()).unwrap_or_default(),
                _ => prof.cwd.clone(),
            });
            items.push(Item {
                title: format!("New tab: {}", prof.name),
                hint: "profile".into(),
                kind: ItemKind::Program(prof.name.clone(), prof.program.clone(), prof.args.clone(), cwd),
            });
        }
        for shell in config::installed_shells() {
            let name = shell.rsplit(['/', '\\']).next().unwrap_or(&shell).trim_end_matches(".exe").to_owned();
            items.push(Item { title: format!("New tab: {name}"), hint: shell.clone(), kind: ItemKind::Program(name, shell, Vec::new(), None) });
        }

        let mut seen = std::collections::HashSet::new();
        for prof in &self.config.ssh {
            seen.insert(prof.host.clone());
            items.push(Item {
                title: format!("SSH: {}", prof.display_name()),
                hint: prof.ssh_args().join(" "),
                kind: ItemKind::Ssh(prof.display_name().to_owned(), prof.ssh_args()),
            });
        }
        if let Some(cfg) = dirs::home_dir().map(|h| h.join(".ssh/config")) {
            for host in zhell_core::ssh::hosts_from_config(&cfg) {
                if seen.insert(host.clone()) {
                    items.push(Item { title: format!("SSH: {host}"), hint: "~/.ssh/config".into(), kind: ItemKind::Ssh(host.clone(), vec![host]) });
                }
            }
        }
        let mut p = palette::Palette::new(items);

        let req = self.next_req;
        self.next_req += 1;
        p.req = req;
        self.host.send(ClientMsg::HistorySearch {
            req,
            query: zhell_proto::HistoryQuery { starred_only: true, limit: 100, ..Default::default() },
        });
        self.palette = Some(p);
        self.search = None;
        self.window.request_redraw();
    }

    fn palette_effect(&mut self, effect: palette::PaletteEffect) {
        use palette::{ItemKind, PaletteEffect as E};
        match effect {
            E::None => {}
            E::Close => self.palette = None,
            E::Run(kind) => {
                self.palette = None;
                match kind {
                    ItemKind::Action(a) => {
                        self.run(a);
                    }
                    ItemKind::Theme(name) => {
                        let cfg = Config { theme: name, ..self.config.clone() };
                        self.theme = load_theme(&cfg, self.config_path.as_deref(), self.system_dark());
                    }
                    ItemKind::Command(cmd) => {
                        let names = template_names(&cmd);
                        if names.is_empty() {
                            self.write(cmd.replace('\n', " ").into_bytes());
                        } else {
                            self.fill_template(cmd, names, Vec::new());
                        }
                    }
                    ItemKind::Program(_name, program, args, cwd) => {
                        let setup = PaneSetup { cwd, ..Default::default() };
                        self.spawn_with(Placement::NewTab, Some((program, args)), setup);
                    }
                    ItemKind::Ssh(name, args) => {
                        let setup = PaneSetup { title: Some(format!("⇄ {name}")), ..Default::default() };
                        let ssh = if cfg!(windows) { "ssh.exe" } else { "ssh" };
                        self.spawn_with(Placement::NewTab, Some((ssh.into(), args)), setup);
                    }
                    ItemKind::SaveWorkspace => {
                        if let Some(project) = self.current_project() {
                            match zhell_core::project::save_workspace(&project.root, &project.workspace()) {
                                Ok(path) => self.show_toast(format!("Saved {}", path.display())),
                                Err(e) => self.show_toast(format!("Couldn't save the workspace: {e}")),
                            }
                        }
                    }
                }
            }
        }
        self.window.request_redraw();
    }

    fn search_effect(&mut self, effect: search::SearchEffect) {
        use search::SearchEffect as E;
        match effect {
            E::None => {}
            E::Query(query) => {
                let req = self.next_req;
                self.next_req += 1;
                if let Some(s) = self.search.as_mut() {
                    s.req = req;
                }
                self.host.send(ClientMsg::HistorySearch { req, query });
            }
            E::Preview(id) => {
                let req = self.next_req;
                self.next_req += 1;
                self.host.send(ClientMsg::HistoryGet { req, id });
            }
            E::Insert(cmd) => {
                self.search = None;

                self.write(cmd.replace('\n', " ").into_bytes());
            }
            E::CopyOutput(text) => self.set_clipboard(text, CopyTarget::Clipboard),
            E::Star(id, starred) => self.host.send(ClientMsg::HistoryStar { id, starred }),
            E::EditNote(id, note) => {
                let body = vec!["A reminder shown with this command in search and the palette.".into()];
                self.dialog = Some(dialog::Dialog::new("Note", body, "Save", dialog::Confirm::SetNote(id)).with_input(note.unwrap_or_default()));
            }
            E::EditTemplate(id, text) => {
                let body = vec![
                    "Mark the parts that change with {{name}}, e.g. git checkout {{branch}}.".into(),
                    "Templates are starred and appear in the palette (Ctrl+Shift+P).".into(),
                ];
                self.dialog = Some(dialog::Dialog::new("Template", body, "Save", dialog::Confirm::SetTemplate(id)).with_input(text));
            }
            E::Close => self.search = None,
        }
        self.window.request_redraw();
    }

    fn block_action(&mut self, pane: PaneId, block: u32, action: draw::BlockAction) {
        let Some(view) = self.panes.get(&pane) else { return };
        let cmd = view.mirror.blocks.iter().find(|b| b.id == block).and_then(|b| b.cmd.clone());
        match action {
            draw::BlockAction::ToggleFold => {
                let folded = view.mirror.folds.iter().any(|f| f.block == block);
                self.host.send(ClientMsg::Fold { pane, block, folded: !folded });
            }
            draw::BlockAction::CopyCommand => {
                if let Some(cmd) = cmd {
                    self.set_clipboard(cmd, CopyTarget::Clipboard);
                }
            }
            draw::BlockAction::CopyOutput => {
                self.host.send(ClientMsg::CopyBlockOutput { pane, block, target: CopyTarget::Clipboard });
            }
            draw::BlockAction::SaveOutput => {
                self.saving = Some(cmd.unwrap_or_else(|| "output".into()));
                self.host.send(ClientMsg::CopyBlockOutput { pane, block, target: CopyTarget::File });
            }
            draw::BlockAction::Rerun => {
                let at_prompt = view.mirror.blocks.last().is_some_and(|b| b.state == zhell_proto::BlockState::Editing);
                if let (Some(cmd), true) = (cmd, at_prompt) {
                    let mut bytes = b"\x05\x15".to_vec();
                    bytes.extend(cmd.replace('\n', "\r").as_bytes());
                    bytes.push(b'\r');
                    if view.mirror.display_offset > 0 {
                        self.host.send(ClientMsg::ScrollToBottom { pane });
                    }
                    self.host.send(ClientMsg::Input { pane, bytes });
                }
            }
        }
    }

    fn left_press(&mut self) {
        let (x, y) = (self.cursor_pos.x as f32, self.cursor_pos.y as f32);
        if let Some(m) = &self.menu {
            let res = m.click(x, y);
            self.menu_result(res);
            return;
        }
        if let Some(d) = &self.dialog {
            let res = d.click(x, y);
            self.dialog_result(res);
            return;
        }
        if self.palette.is_some() {
            let effect = self.palette.as_mut().map(|p| p.click(x, y));
            if let Some(e) = effect {
                self.palette_effect(e);
            }
            return;
        }
        if self.search.is_some() {
            let now = Instant::now();
            let double = self.last_click.is_some_and(|(t, ..)| now - t < MULTI_CLICK);
            self.last_click = Some((now, PaneId(0), (0, 0), 1));
            let effect = self.search.as_mut().map(|s| s.click(x, y, double));
            if let Some(e) = effect {
                self.search_effect(e);
            }
            return;
        }
        if let Some((pane, b)) = self.block_buttons.iter().find(|(_, b)| b.rect.contains(x, y)).copied() {
            self.block_action(pane, b.block, b.action);
            return;
        }
        if self.modifiers.control_key()
            && let Some(link) = self.link_at(self.cursor_pos)
        {
            self.open_link(link);
            return;
        }
        if let Some(dir) = self.resize_edge(self.cursor_pos) {
            let _ = self.window.drag_resize_window(dir);
            return;
        }
        if let Some(hit) = self.title_hit(self.cursor_pos) {
            self.title_click(hit);
            return;
        }
        if let Some(d) = self.divider_at(self.cursor_pos) {
            self.drag = Some(Drag::Divider(d));
            return;
        }
        let Some((pane, rect)) = self.pane_at(self.cursor_pos) else { return };

        if self.layout.focused() != Some(pane) {
            self.with_layout(|l, _, _| {
                if let Some(t) = l.active_tab_mut() {
                    t.focus = pane;
                }
            });
        }
        if self.report_mouse(Some(MouseButton::Left), true, false) {
            self.mouse_down = Some(MouseButton::Left);
            return;
        }

        let (row, _, _) = self.cell_in(pane, rect, self.cursor_pos);
        if let Some(f) = self.panes.get(&pane).and_then(|v| v.mirror.folds.iter().find(|f| f.row as i32 == row).copied()) {
            self.host.send(ClientMsg::Fold { pane, block: f.block, folded: false });
            return;
        }
        let rows = self.panes.get(&pane).map_or(1, |p| p.mirror.rows);
        let (row, col, right_half) = self.cell_in(pane, rect, self.cursor_pos);
        let row = row.clamp(0, rows.saturating_sub(1) as i32) as u16;
        let now = Instant::now();
        let count = match self.last_click {
            Some((t, p, cell, n)) if now - t < MULTI_CLICK && p == pane && cell == (row, col) => n % 3 + 1,
            _ => 1,
        };
        self.last_click = Some((now, pane, (row, col), count));
        let kind = match count {
            2 => SelectKind::Word,
            3 => SelectKind::Line,
            _ if self.modifiers.alt_key() => SelectKind::Block,
            _ => SelectKind::Simple,
        };
        self.host.send(ClientMsg::SelectStart { pane, row, col, right_half, kind });

        self.drag = Some(Drag::Select { pane, moved: count > 1 });
    }

    fn left_release(&mut self) {
        match self.drag.take() {
            Some(Drag::Select { pane, moved: true }) => {
                self.host.send(ClientMsg::Copy { pane, target: CopyTarget::Primary });
            }
            Some(Drag::Select { pane, moved: false }) => self.host.send(ClientMsg::SelectClear { pane }),
            Some(Drag::Divider(_) | Drag::Tab(_)) => {}
            None => {
                if self.mouse_down.take().is_some() {
                    self.report_mouse(Some(MouseButton::Left), false, false);
                }
            }
        }
    }

    fn pointer_moved(&mut self, position: PhysicalPosition<f64>) {
        let before = self.cursor_pos;
        self.cursor_pos = position;
        if let Some(m) = self.menu.as_mut() {
            if m.hover(position.x as f32, position.y as f32) {
                self.window.request_redraw();
            }
            return;
        }
        if let Some(p) = self.palette.as_mut() {
            if p.hover(position.x as f32, position.y as f32) {
                self.window.request_redraw();
            }
            return;
        }
        match self.drag {
            Some(Drag::Divider(d)) => {
                let a = d.area;
                let ratio = match d.dir {
                    SplitDir::Horizontal => (position.x as f32 - a.x) / a.w,
                    SplitDir::Vertical => (position.y as f32 - a.y) / a.h,
                };
                if let Some(t) = self.layout.active_tab_mut() {
                    t.root.set_ratio(d.path, d.depth, ratio);
                }

                self.relayout();
            }
            Some(Drag::Tab(from)) => {
                let l = self.title_layout();
                let x = position.x as f32;

                let to = l.tabs.iter().position(|r| x < r.x + r.w).unwrap_or(l.tabs.len().saturating_sub(1));
                if to != from {
                    self.layout.move_tab(from, to);
                    self.drag = Some(Drag::Tab(to));
                    self.store_layout();
                    self.window.request_redraw();
                }
            }
            Some(Drag::Select { pane, .. }) => {
                let Some(rect) = self.rect_of(pane) else { return };
                let now = self.cell_in(pane, rect, position);
                if now == self.cell_in(pane, rect, before) {
                    return;
                }
                let (row, col, right_half) = now;
                if let Some(view) = self.panes.get(&pane) {
                    if row < 0 && view.mirror.modes & mode::ALT_SCREEN == 0 {
                        self.host.send(ClientMsg::Scroll { pane, delta: 1 });
                    } else if row >= view.mirror.rows as i32 && view.mirror.display_offset > 0 {
                        self.host.send(ClientMsg::Scroll { pane, delta: -1 });
                    }
                }
                self.host.send(ClientMsg::SelectUpdate { pane, row, col, right_half });
                self.drag = Some(Drag::Select { pane, moved: true });
            }
            None if self.modifiers.control_key() => self.update_hover(),
            None => {
                self.update_block_hover();
                let hit = self.title_hit(position);
                if hit != self.title_hover {
                    self.title_hover = hit;
                    self.window.request_redraw();
                }
                let icon = if let Some(dir) = self.resize_edge(position) {
                    CursorIcon::from(dir)
                } else {
                    match (self.divider_at(position).map(|d| d.dir), hit) {
                        (Some(SplitDir::Horizontal), _) => CursorIcon::ColResize,
                        (Some(SplitDir::Vertical), _) => CursorIcon::RowResize,
                        (None, Some(_)) => CursorIcon::Default,
                        (None, None) => CursorIcon::Text,
                    }
                };
                self.window.set_cursor(icon);
                let cell = |pos| self.pane_at(pos).map(|(p, r)| (p, self.cell_in(p, r, pos)));
                let (a, b) = (cell(before), cell(position));
                if a.map(|(p, c)| (p, c.0, c.1)) != b.map(|(p, c)| (p, c.0, c.1)) {
                    self.report_mouse(self.mouse_down, true, true);
                }
            }
        }
    }

    fn system_dark(&self) -> bool {
        match self.window.theme() {
            Some(t) => t == winit::window::Theme::Dark,
            None => zhell_core::appearance::prefers_dark(),
        }
    }

    fn follow_system_theme(&mut self) {
        if self.config.theme != "auto" {
            return;
        }
        let theme = load_theme(&self.config, self.config_path.as_deref(), self.system_dark());
        if theme.background != self.theme.background || theme.foreground != self.theme.foreground {
            self.theme = theme;
            self.window.request_redraw();
        }
    }

    fn opacity(&self) -> f32 {
        self.opacity_override.unwrap_or(self.config.window.opacity)
    }

    fn set_opacity(&mut self, value: Option<f32>) {
        if !self.renderer.transparent() {
            self.show_error("This window can't be see-through: set [window] opacity below 1 (or use the custom title bar) and reopen it".into());
            return;
        }
        self.opacity_override = value.map(|v| (v * 100.0).round() / 100.0).filter(|v| (*v - self.config.window.opacity).abs() > 0.001);
        let pct = (self.opacity() * 100.0).round();
        self.show_toast(match self.opacity_override {
            Some(v) => format!("Opacity {pct}% — keep it with  [window] opacity = {v}"),
            None => format!("Opacity {pct}% (from your settings)"),
        });
        self.update_glass();
        self.window.request_redraw();
    }

    fn wheel(&mut self, lines: f64) {
        if self.modifiers.control_key() && self.modifiers.shift_key() {
            let step = if lines > 0.0 { 0.05 } else { -0.05 };
            self.set_opacity(Some((self.opacity() + step).clamp(0.3, 1.0)));
            return;
        }
        if let Some(p) = self.palette.as_mut() {
            p.wheel(lines.round() as i32);
            self.window.request_redraw();
            return;
        }
        if let Some(s) = self.search.as_mut() {
            s.wheel(lines.round() as i32);
            self.window.request_redraw();
            return;
        }
        let Some((pane, _)) = self.pane_at(self.cursor_pos) else { return };
        let Some(view) = self.panes.get_mut(&pane) else { return };
        view.scroll_acc += lines;
        let n = view.scroll_acc.trunc() as i32;
        if n == 0 {
            return;
        }
        view.scroll_acc -= n as f64;
        let modes = view.mirror.modes;
        let up = n > 0;
        if modes & (mode::MOUSE_REPORT_CLICK | mode::MOUSE_DRAG | mode::MOUSE_MOTION) != 0
            && !self.modifiers.shift_key()
        {
            let btn = if up { MouseButton::WheelUp } else { MouseButton::WheelDown };
            for _ in 0..n.abs() {
                self.report_mouse(Some(btn), true, false);
            }
        } else if modes & mode::ALT_SCREEN != 0 {
            if modes & mode::ALTERNATE_SCROLL != 0 {
                let seq: &[u8] = match (up, modes & mode::APP_CURSOR != 0) {
                    (true, true) => b"\x1bOA",
                    (true, false) => b"\x1b[A",
                    (false, true) => b"\x1bOB",
                    (false, false) => b"\x1b[B",
                };
                self.host.send(ClientMsg::Input { pane, bytes: seq.repeat(n.unsigned_abs() as usize) });
            }
        } else {
            self.host.send(ClientMsg::Scroll { pane, delta: n });
            self.mark_user_scroll(pane);
        }
    }

    fn mark_user_scroll(&mut self, pane: PaneId) {
        if let Some(v) = self.panes.get_mut(&pane) {
            v.user_scrolled = true;
        }
    }

    fn scroll_page(&mut self, up: bool) {
        let Some(p) = self.focused_pane() else { return };
        if p.mirror.modes & mode::ALT_SCREEN != 0 {
            return;
        }
        let page = p.mirror.rows.saturating_sub(1).max(1) as i32;
        let pane = p.id;
        self.host.send(ClientMsg::Scroll { pane, delta: if up { page } else { -page } });
        self.mark_user_scroll(pane);
    }

    fn shortcut(&mut self, event: &winit::event::KeyEvent) -> bool {
        let Some(action) = input::combos(event, self.modifiers).iter().find_map(|c| self.keymap.get(c)) else {
            return false;
        };
        self.run(action)
    }

    fn run(&mut self, action: Action) -> bool {
        let multi_pane = self.layout.active_tab().is_some_and(|t| t.root.panes().len() > 1);
        match action {
            Action::Copy => self.copy(),
            Action::Paste => self.paste(),
            Action::NewTab => self.spawn(Placement::NewTab, None),
            Action::ClosePane => self.close_focused(),
            Action::SplitRight => self.spawn(Placement::Split(SplitDir::Horizontal), None),
            Action::SplitDown => self.spawn(Placement::Split(SplitDir::Vertical), None),
            Action::ToggleZoom => self.with_layout(|l, _, _| {
                if let Some(t) = l.active_tab_mut() {
                    t.zoomed = !t.zoomed && t.root.panes().len() > 1;
                }
            }),
            Action::NextTab | Action::PrevTab if self.layout.tabs.len() > 1 => {
                let fwd = action == Action::NextTab;
                self.with_layout(|l, _, _| l.cycle_tab(fwd));
            }
            Action::FocusLeft | Action::FocusRight | Action::FocusUp | Action::FocusDown if multi_pane => {
                let dir = match action {
                    Action::FocusLeft => Direction::Left,
                    Action::FocusRight => Direction::Right,
                    Action::FocusUp => Direction::Up,
                    _ => Direction::Down,
                };
                self.with_layout(|l, area, gap| l.move_focus(area, gap, dir));
            }
            Action::FontBigger => self.set_font_size(self.font_size + 1.0),
            Action::FontSmaller => self.set_font_size(self.font_size - 1.0),
            Action::FontReset => self.set_font_size(self.config.font.size),
            Action::ScrollPageUp => self.scroll_page(true),
            Action::ScrollPageDown => self.scroll_page(false),
            Action::CommandPalette => self.open_palette(),
            Action::OpenSettings => self.open_settings(),
            Action::RenameTab => self.rename_tab(self.layout.active),
            Action::NewWindow => self.open_window(&["--new-window".to_owned()]),
            Action::About => {
                let body = vec![
                    format!("Version {} · GPL-3.0-or-later", env!("CARGO_PKG_VERSION")),
                    "A fast GPU terminal that remembers every command and never loses a session.".into(),
                    format!("Made by TheHolyOneZ · {}", env!("CARGO_PKG_HOMEPAGE")),
                    "More projects: https://zsync.eu".into(),
                ];
                let mut d = dialog::Dialog::new("Zhell", body, "Website", dialog::Confirm::OpenUrl(env!("CARGO_PKG_HOMEPAGE").into()))
                    .with_alt("All projects", dialog::Confirm::OpenUrl("https://zsync.eu".into()))
                    .default_confirm();
                d.cancel_label = "Close".into();
                self.dialog = Some(d);
                self.window.request_redraw();
            }
            Action::CopyMode => {
                let Some(pane) = self.layout.focused() else { return false };
                self.copy_mode = Some(copymode::CopyMode::new(pane));
                self.host.send(ClientMsg::CopyMode { pane, cmd: zhell_proto::CopyCmd::Enter });
                self.window.request_redraw();
            }
            Action::QuickSelect => {
                let Some(p) = self.focused_pane() else { return false };
                self.quick = quickselect::QuickSelect::new(p.id, &p.mirror.lines, &self.config.quick_select.patterns);
                if self.quick.is_none() {
                    self.show_toast("Nothing to pick on screen (URLs, paths, hashes, IPs, numbers)".into());
                }
                self.window.request_redraw();
            }
            Action::OpacityDown | Action::OpacityUp | Action::OpacityReset => {
                let o = match action {
                    Action::OpacityReset => None,
                    Action::OpacityUp => Some((self.opacity() + 0.05).min(1.0)),
                    _ => Some((self.opacity() - 0.05).max(0.3)),
                };
                self.set_opacity(o);
            }
            Action::ResizeLeft | Action::ResizeRight | Action::ResizeUp | Action::ResizeDown => {
                let dir = match action {
                    Action::ResizeLeft => Direction::Left,
                    Action::ResizeRight => Direction::Right,
                    Action::ResizeUp => Direction::Up,
                    _ => Direction::Down,
                };
                let m = self.renderer.cell_metrics();
                let step = if matches!(dir, Direction::Left | Direction::Right) { m.width * 4.0 } else { m.height * 2.0 };
                self.with_layout(|l, area, gap| {
                    l.resize_focused(area, gap, dir, step);
                });
                self.store_layout();
            }
            Action::ToggleRecording => {
                let Some(p) = self.focused_pane() else { return false };
                let path = match p.recording {
                    Some(_) => None,
                    None => Some(recording_path(&p.label()).display().to_string()),
                };
                self.host.send(ClientMsg::Record { pane: p.id, path });
            }
            Action::ToggleBroadcast => {
                if self.broadcast_targets().is_some() {
                    self.broadcast = None;
                    self.show_toast("Broadcast off".into());
                } else if multi_pane {
                    self.broadcast = self.layout.focused();
                    let n = self.layout.active_tab().map_or(0, |t| t.root.panes().len());
                    self.show_toast(format!("Typing goes to all {n} panes of this tab — {} again to stop", self.keymap.key_for(Action::ToggleBroadcast).map(|k| k.to_string()).unwrap_or_else(|| "toggle".into())));
                } else {
                    self.show_error("Broadcast needs a split: it types into every pane of the tab".into());
                }
            }
            Action::InstallRemoteIntegration => {
                let Some(pane) = self.layout.focused() else { return false };
                let req = self.next_req;
                self.next_req += 1;
                self.pending_install = Some((req, pane));
                self.host.send(ClientMsg::QueryForeground { req, pane });
            }
            Action::MoveToNewWindow => {
                let Some(pane) = self.layout.focused() else { return false };
                if !self.host.shared() {
                    self.show_error("Moving panes between windows needs the session daemon ([sessions] daemon = true)".into());
                    return true;
                }
                self.open_window(&["--attach".to_owned(), pane.0.to_string()]);
            }
            Action::Find => {
                let Some(pane) = self.layout.focused() else { return false };
                if let Some(fb) = self.findbar.take() {
                    self.host.send(ClientMsg::FindClose { pane: fb.pane });
                }
                self.findbar = Some(findbar::FindBar::new(pane));
                self.window.request_redraw();
            }
            Action::OpenWorkspace => {
                let Some(project) = self.current_project() else { return false };
                self.open_workspace(&project);
            }
            Action::ToggleScreenShare => {
                self.screen_share_manual = Some(!self.screen_sharing());
                self.window.request_redraw();
            }
            Action::SearchHistory => {
                let mut s = search::SearchOverlay::new();
                let q = s.query();
                s.req = self.next_req;
                self.search = Some(s);
                self.search_effect(search::SearchEffect::Query(q));
            }
            Action::PrevBlock | Action::NextBlock => {
                let Some(p) = self.focused_pane() else { return false };
                if p.mirror.modes & mode::ALT_SCREEN != 0 || p.mirror.blocks.is_empty() {
                    return false;
                }
                let pane = p.id;
                self.host.send(ClientMsg::JumpBlock { pane, forward: action == Action::NextBlock });
                self.mark_user_scroll(pane);
            }
            Action::ScrollTop | Action::ScrollBottom => {
                let Some(p) = self.focused_pane() else { return false };
                if p.mirror.modes & mode::ALT_SCREEN != 0 {
                    return false;
                }
                let msg = if action == Action::ScrollTop {
                    ClientMsg::Scroll { pane: p.id, delta: p.mirror.history_len.min(i32::MAX as u32) as i32 }
                } else {
                    ClientMsg::ScrollToBottom { pane: p.id }
                };
                let pane = p.id;
                self.host.send(msg);
                self.mark_user_scroll(pane);
            }
            a => match a.select_tab_index() {
                Some(i) if i < self.layout.tabs.len() && self.layout.tabs.len() > 1 => {
                    self.with_layout(|l, _, _| l.select_tab(i))
                }
                _ => return false,
            },
        }
        true
    }

    fn pump(&mut self) -> bool {
        let mut redraw = false;
        while let Some(msg) = self.host.try_recv() {
            match msg {
                ServerMsg::HelloOk { restore, host_version, .. } => {
                    log::info!("session host {host_version}");
                    if let Some(r) = restore {
                        self.restore(r);
                    }
                    if let Some(pane) = self.attach_pane.take() {
                        self.adopt(pane);
                    } else if self.panes.is_empty() || self.cli_program.is_some() {
                        let program = self.cli_program.take();
                        self.spawn(Placement::NewTab, program);
                    }
                }
                ServerMsg::VersionMismatch { host_proto_version } => {
                    log::error!("session host speaks protocol {host_proto_version}, we speak {PROTO_VERSION}");
                    return false;
                }
                ServerMsg::PaneCreated { req, pane } => self.placed(req, pane),
                ServerMsg::SpawnFailed { req, error } => {
                    log::error!("failed to start shell: {error}");
                    self.pending.remove(&req);
                    if self.panes.is_empty() {
                        return false;
                    }
                }
                ServerMsg::Frame(f) => {
                    self.notify_finished(&f);
                    let focused = self.layout.focused() == Some(f.pane);
                    let cell_h = self.renderer.cell_metrics().height;
                    let smooth = self.config.window.smooth_scroll && !reduce_motion();
                    if let Some(p) = self.panes.get_mut(&f.pane) {
                        let moved = (f.cursor.row, f.cursor.col) != (p.mirror.cursor.row, p.mirror.cursor.col);

                        let delta = (f.display_offset as i64 - p.mirror.display_offset as i64)
                            - (f.history_len as i64 - p.mirror.history_len as i64);
                        let animate = smooth
                            && std::mem::take(&mut p.user_scrolled)
                            && delta != 0
                            && delta.unsigned_abs() < p.mirror.rows as u64
                            && f.folds.is_empty()
                            && p.mirror.folds.is_empty()
                            && f.modes & mode::ALT_SCREEN == 0;
                        if animate {
                            let d = delta as f32 * cell_h;
                            let carried = p.scroll_anim.as_ref().map_or(0.0, |a| a.shift);
                            p.scroll_anim = Some(view::ScrollAnim {
                                shift: carried - d,
                                prev_offset: d,
                                prev: p.mirror.clone(),
                                last: Instant::now(),
                            });
                        }
                        p.mirror.apply(f);
                        if moved && focused {
                            self.reset_blink();
                        }
                        redraw = true;
                    }
                }
                ServerMsg::Title { pane, title } => {
                    if let Some(p) = self.panes.get_mut(&pane) {
                        p.title = title;
                    }
                    self.update_title();
                    redraw = true;
                }
                ServerMsg::Cwd { pane, cwd } => {
                    let changed = self.panes.get(&pane).is_some_and(|p| p.cwd.as_deref() != Some(&cwd));
                    if let Some(p) = self.panes.get_mut(&pane) {
                        p.cwd = Some(cwd);

                        p.remote = None;
                    }
                    if changed && self.layout.focused() == Some(pane) {
                        self.cwd_changed();
                    }
                    self.update_title();
                    redraw = true;
                }

                ServerMsg::Ready { pane } => {
                    if let Some((p, until)) = self.install_waiting
                        && p == pane
                        && Instant::now() < until
                    {
                        self.install_waiting = None;
                        self.host.send(ClientMsg::Input { pane, bytes: remote::installer_payload().into_bytes() });
                    }
                }
                ServerMsg::Quake => self.toggle_quake(),
                ServerMsg::QuakeHandled { .. } => {}
                ServerMsg::BlockCells { req, rows } => {
                    if let Some((r, cmd)) = self.pending_export.take() {
                        if r == req {
                            self.save_html(&cmd, &rows);
                        } else {
                            self.pending_export = Some((r, cmd));
                        }
                    }
                }
                ServerMsg::Recording { pane, path, done, error } => {
                    if let Some(e) = error {
                        self.show_error(format!("Couldn't record — {e}"));
                    } else if done {
                        if let Some(path) = &path {
                            self.show_toast(format!("Recording saved: {path}  (asciinema play to watch)"));
                        }
                    } else if let Some(path) = &path {
                        self.show_toast(format!("Recording to {path}"));
                    }
                    if let Some(p) = self.panes.get_mut(&pane) {
                        p.recording = if done { None } else { path };
                    }
                    redraw = true;
                }
                ServerMsg::RemoteCwd { pane, host, cwd } => {
                    if let Some(p) = self.panes.get_mut(&pane) {
                        p.remote = Some((host, cwd));
                    }
                    self.update_title();
                    redraw = true;
                }
                ServerMsg::Clipboard { text, .. } => self.set_clipboard(text, CopyTarget::Clipboard),
                ServerMsg::CopyText { text, target, .. } => self.set_clipboard(text, target),
                ServerMsg::PathResolved { req, path } => {
                    let pos = self.pending_paths.remove(&req);
                    match (path, pos) {
                        (Some(path), Some((line, col))) => self.open_file(&path, line, col),
                        (None, _) => log::info!("link target does not exist"),
                        _ => {}
                    }
                }
                ServerMsg::Bell { .. } => {
                    if !self.focused {
                        self.window.request_user_attention(Some(winit::window::UserAttentionType::Informational));
                    }
                }
                ServerMsg::Exited { pane, code } => {
                    if !self.pane_exited(pane) {
                        log::info!("the last shell ended (exit code {code:?}); closing the window");
                        return false;
                    }
                }

                ServerMsg::Detached { pane } => {
                    if !self.pane_exited(pane) {
                        self.closing_keep = Some(true);
                        return false;
                    }
                    self.store_layout();
                }
                ServerMsg::HistoryResults { req, hits } if self.palette.as_ref().is_some_and(|p| p.req == req) => {
                    let items = hits.into_iter().map(|h| {
                        let text = h.template.clone().unwrap_or_else(|| h.cmd.clone());
                        palette::Item {
                            title: h.note.clone().map_or_else(|| text.clone(), |n| format!("{text} — {n}")),
                            hint: if h.template.is_some() { "⧉ template".into() } else { "★ starred".into() },
                            kind: palette::ItemKind::Command(text),
                        }
                    });
                    if let Some(p) = self.palette.as_mut() {
                        p.add_items(items);
                    }
                    redraw = true;
                }
                ServerMsg::HistoryResults { req, hits } => {
                    let effect = match self.search.as_mut() {
                        Some(s) if s.req == req => Some(s.set_results(hits)),
                        _ => None,
                    };
                    if let Some(e) = effect {
                        self.search_effect(e);
                    }
                    redraw = true;
                }
                ServerMsg::HistoryEntry { entry, .. } => {
                    if let (Some(s), Some((hit, output))) = (self.search.as_mut(), entry) {
                        s.preview = Some((hit.id, output));
                    }
                    redraw = true;
                }
                ServerMsg::Foreground { req, name } if self.pending_install.is_some_and(|(r, _)| r == req) => {
                    let Some((_, pane)) = self.pending_install.take() else { continue };
                    self.offer_remote_install(pane, name.as_deref());
                }
                ServerMsg::Foreground { req, name } => {
                    if let Some((r, text)) = self.pending_paste.take() {
                        if r == req {
                            self.paste_checked(text, name.as_deref());
                        } else {
                            self.pending_paste = Some((r, text));
                        }
                    }
                }
                ServerMsg::Image { pane, id, width, height, rgba, .. } => {
                    if let Some(p) = self.panes.get_mut(&pane) {
                        p.images.insert(id, view::ImagePixels { width, height, rgba });

                        self.renderer.forget_image((pane.0 << 32) | id as u64);
                    }
                    redraw = true;
                }
                ServerMsg::Ports { pane, ports } => {
                    if let Some(p) = self.panes.get_mut(&pane) {
                        p.ports = ports;
                    }
                    redraw = true;
                }
                ServerMsg::Background { count } => {
                    self.background = count;
                    redraw = true;
                }
                ServerMsg::Mark { pane, mark } => {
                    if mark.kind == zhell_proto::MarkKind::PromptStart
                        && let Some((input, _)) = self.pending_input.remove(&pane)
                    {
                        self.host.send(ClientMsg::Input { pane, bytes: format!("{input}\r").into_bytes() });
                    }
                }
            }
        }
        if redraw {
            self.window.request_redraw();
        }
        if !self.host.is_alive() {
            log::error!("the session host went away");
            return self.reconnect();
        }
        true
    }

    fn step_cursor_animation(&mut self, visible: &[(PaneId, Rect)]) -> (f32, f32) {
        let Some(focus) = self.layout.focused() else { return (0.0, 0.0) };
        let Some((_, rect)) = visible.iter().find(|(id, _)| *id == focus) else { return (0.0, 0.0) };
        let Some(view) = self.panes.get(&focus) else { return (0.0, 0.0) };
        let m = self.renderer.cell_metrics();
        let pad = self.padding();
        let c = view.mirror.cursor;
        let target = (rect.x + pad + c.col as f32 * m.width, rect.y + pad + c.row as f32 * m.height);
        let now = Instant::now();
        let (x, y) = match self.cursor_anim {
            Some((p, x, y, last))
                if p == focus
                    && self.config.cursor.smooth
                    && !reduce_motion()
                    && (y - target.1).abs() < m.height * 10.0 =>
            {
                let dt = now.duration_since(last).as_secs_f32().min(0.05);
                let k = 1.0 - (-dt * 45.0).exp();
                (x + (target.0 - x) * k, y + (target.1 - y) * k)
            }
            _ => target,
        };
        let (dx, dy) = (x - target.0, y - target.1);
        let settled = dx.abs() < 0.5 && dy.abs() < 0.5;
        let (x, y) = if settled { target } else { (x, y) };
        self.cursor_anim = Some((focus, x, y, now));
        if !settled {
            self.animating = true;
        }
        if settled { (0.0, 0.0) } else { (dx, dy) }
    }

    fn draw(&mut self) {
        self.update_glass();

        for attempt in 0..3 {
            self.build_frame();

            match self.renderer.render([0.0; 4]) {
                FrameOutcome::Rebuild if attempt < 2 => continue,
                FrameOutcome::Rebuild => break,
                FrameOutcome::Presented | FrameOutcome::Skipped => break,
            }
        }
        if self.animating {
            self.window.request_redraw();
        }

        for p in self.panes.values_mut() {
            if let Some(seq) = p.mirror.last_seq.take() {
                self.host.send(ClientMsg::Ack { pane: p.id, seq });
            }
        }
    }

    fn build_frame(&mut self) {
        let blink_hidden = self.cursor_blinks() && !self.blink_on;
        let visible = self.visible();
        let focus = self.layout.focused();
        let pad = self.padding();
        let split = visible.len() > 1;
        let show_bar = self.show_tab_bar();
        let title_layout = self.title_layout();
        let tabs: Vec<draw::TabInfo> = if show_bar { self.tab_infos() } else { Vec::new() };
        let dividers = match self.layout.active_tab() {
            Some(t) if !t.zoomed => t.root.dividers(self.content_area(), self.gap()),
            _ => Vec::new(),
        };

        let size = self.window.inner_size();
        let (win_w, win_h) = (size.width as f32, size.height as f32);
        let rounded = self.custom_frame() && !self.window.is_maximized() && self.window.fullscreen().is_none() && !self.quake;
        let radius = if rounded { (theme::RADIUS * self.scale).round() } else { 0.0 };
        let was_animating = self.animating;
        self.animating = false;

        let now = Instant::now();

        let dt = if was_animating { now.duration_since(self.anim_last).as_secs_f32().min(0.05) } else { 1.0 / 60.0 };
        self.anim_last = now;
        let instant = reduce_motion();
        for p in self.panes.values_mut() {
            if let Some(a) = p.scroll_anim.as_mut() {
                let dt = now.duration_since(a.last).as_secs_f32().min(0.05);
                a.last = now;
                a.shift *= (-dt * 22.0).exp();
                if a.shift.abs() < 0.5 {
                    p.scroll_anim = None;
                } else {
                    self.animating = true;
                }
            }
        }
        let sharing = self.screen_sharing();
        let broadcast = self.broadcast_targets().unwrap_or_default();
        let cursor_offset = self.step_cursor_animation(&visible);
        let opacity = self.opacity();
        let r = &mut self.renderer;
        r.set_window_radius(radius);
        r.begin();
        let mut base = self.theme.background;
        if r.transparent() {
            base[3] = opacity;
        }
        if r.has_background() {
            r.set_background_tint(base);
        } else {
            r.rect(0.0, 0.0, win_w, win_h, base);
        }
        let mut buttons = Vec::new();
        for (id, rect) in &visible {
            let Some(view) = self.panes.get(id) else { continue };
            let is_focus = focus == Some(*id);
            let style = draw::PaneStyle {
                origin: (rect.x + pad, rect.y + pad),
                area: *rect,
                scale: self.scale,
                focused: is_focus && self.focused,
                cursor_visible: !(is_focus && blink_hidden),
                dim: split && !is_focus,
                cursor_offset: if is_focus { cursor_offset } else { (0.0, 0.0) },
                redact: sharing,
                pane_key: id.0,
                ligatures: self.config.font.ligatures,
                link: self.hover_link.as_ref().filter(|l| l.pane == *id).map(|l| l.cells.clone()).unwrap_or_default(),
                bg_alpha: self.config.window.text_background_opacity,
            };

            let shift = view.scroll_anim.as_ref().map_or(0.0, |a| a.shift);
            if let Some(a) = &view.scroll_anim {
                r.clip(Some([rect.x, rect.y, rect.w, rect.h]));
                let rows_h = view.mirror.rows as f32 * r.cell_metrics().height;
                let (sy, sh) = if shift < 0.0 {
                    let top = (style.origin.1 + shift + rows_h).max(rect.y);
                    (top, rect.y + rect.h - top)
                } else {
                    (rect.y, (style.origin.1 + shift - rect.y).max(0.0))
                };
                r.clip(Some([rect.x, sy, rect.w, sh]));
                let prev_style = draw::PaneStyle { origin: (style.origin.0, style.origin.1 + shift + a.prev_offset), cursor_visible: false, ..style.clone() };
                draw::pane(r, &self.theme, &a.prev, &prev_style);
                r.clip(Some([rect.x, rect.y, rect.w, rect.h]));
            }
            let style = draw::PaneStyle { origin: (style.origin.0, style.origin.1 + shift), ..style };
            draw::pane(r, &self.theme, &view.mirror, &style);
            draw::images(r, &view.mirror, &view.images, &style);
            if let Some(q) = self.quick.as_ref().filter(|q| q.pane == *id) {
                q.draw(r, &self.theme, style.origin, self.scale);
            }
            if self.copy_mode.as_ref().is_some_and(|c| c.pane == *id) {
                draw::mode_badge(r, &self.theme, *rect, "COPY", "v select · y copy · esc quit", self.scale);
            }
            let hovered = self.hovered_block.filter(|(p, _)| p == id).map(|(_, b)| b);
            for b in draw::blocks(r, &self.theme, &view.mirror, &style, hovered) {
                buttons.push((*id, b));
            }
            if view.scroll_anim.is_some() {
                r.clip(None);
            }

            if broadcast.contains(id) {
                let c = self.theme.palette[3];
                let t = (2.0 * self.scale).round();
                r.rounded_outline([rect.x, rect.y, rect.w, rect.h], theme::RADIUS_SMALL * self.scale, t, [c[0], c[1], c[2], 0.9]);
            }
        }

        let cursor_px = focus.and_then(|f| visible.iter().find(|(id, _)| *id == f)).and_then(|(id, rect)| {
            let c = self.panes.get(id)?.mirror.cursor;
            let m = r.cell_metrics();
            Some((rect.x + pad + c.col as f32 * m.width, rect.y + pad + c.row as f32 * m.height))
        });
        if let (Some(text), Some((x, y))) = (&self.preedit, cursor_px) {
            draw::preedit(r, &self.theme, text, x, y, self.scale);
        }
        if let Some((x, y)) = cursor_px {
            let area = (x as i32, y as i32);
            if self.ime_area != Some(area) {
                let m = r.cell_metrics();
                self.window.set_ime_cursor_area(
                    PhysicalPosition::new(area.0, area.1),
                    PhysicalSize::new(m.width as u32, m.height as u32),
                );
                self.ime_area = Some(area);
            }
        }
        let divider_color = {
            let (b, f) = (self.theme.background, self.theme.foreground);
            [b[0] + (f[0] - b[0]) * 0.15, b[1] + (f[1] - b[1]) * 0.15, b[2] + (f[2] - b[2]) * 0.15, 1.0]
        };
        for d in &dividers {
            r.rect(d.rect.x, d.rect.y, d.rect.w, d.rect.h, divider_color);
        }
        if show_bar {
            let active = tabs.iter().zip(&title_layout.tabs).find(|(t, _)| t.active).map(|(_, r)| *r);
            let slid = match (active, self.tab_slide.as_mut()) {
                (Some(a), Some(s)) => {
                    s.to(a.x, a.w);
                    self.animating |= s.step(dt, instant);
                    Some(Rect { x: s.x.value, w: s.w.value, ..a })
                }
                (Some(a), None) => {
                    self.tab_slide = Some(anim::Slide::new(a.x, a.w));
                    Some(a)
                }
                (None, _) => {
                    self.tab_slide = None;
                    None
                }
            };
            let st = titlebar::State {
                buttons: self.config.window.buttons,
                active: slid,
                hover: self.title_hover,
                focused: self.focused,
                maximized: self.window.is_maximized(),
                background: self.background as usize,
                sharing,
                opacity: if r.transparent() { opacity } else { 1.0 },
            };
            titlebar::draw(r, &self.theme, &title_layout, &tabs, &st, self.scale);
        }
        if let Some(fb) = &self.findbar
            && let Some((_, rect)) = visible.iter().find(|(id, _)| *id == fb.pane)
        {
            let state = self.panes.get(&fb.pane).and_then(|p| p.mirror.find.as_ref());
            fb.draw(r, &self.theme, *rect, state, self.scale);
        }

        let toast_text = self.toast.as_ref().filter(|t| Instant::now() < t.1).map(|t| t.0.clone());
        if toast_text.as_ref() != Some(&self.toast_shown) {
            self.appear.remove("toast");
            self.toast_shown = toast_text.clone().unwrap_or_default();
        }
        let present = [
            ("search", self.search.is_some(), anim::SOFT, 14.0),
            ("palette", self.palette.is_some(), anim::SOFT, 14.0),
            ("toast", toast_text.is_some(), anim::DEFAULT, 18.0),
            ("dialog", self.dialog.is_some(), anim::SOFT, 14.0),
            ("menu", self.menu.is_some(), anim::SNAPPY, -6.0),
        ];
        let mut fades: HashMap<&str, (f32, f32)> = HashMap::new();
        for (key, shown, params, rise) in present {
            if !shown {
                self.appear.remove(key);
                continue;
            }
            let s = self.appear.entry(key).or_insert_with(|| anim::Spring::new(0.0, 1.0, params));
            self.animating |= s.step(dt, instant);
            let v = s.value;
            fades.insert(key, (v.min(1.0), ((1.0 - v) * rise * self.scale).round()));
        }
        let fade = |r: &mut zhell_render::Renderer, key: &str| {
            let (a, dy) = fades.get(key).copied().unwrap_or((1.0, 0.0));
            r.set_fade(a, dy);
        };
        if let Some(s) = self.search.as_mut() {
            let area = Rect { x: 0.0, y: 0.0, w: win_w, h: win_h };
            fade(r, "search");
            s.draw(r, &self.theme, area, self.scale);
        }
        if let Some(p) = self.palette.as_mut() {
            let area = Rect { x: 0.0, y: 0.0, w: win_w, h: win_h };
            fade(r, "palette");
            p.draw(r, &self.theme, area, self.scale);
        }
        if let Some((text, until, error)) = &self.toast
            && Instant::now() < *until
        {
            fade(r, "toast");
            draw::toast(r, &self.theme, text, *error, win_w, win_h, self.scale);
        }
        if let Some(d) = self.dialog.as_mut() {
            let area = Rect { x: 0.0, y: 0.0, w: win_w, h: win_h };
            fade(r, "dialog");
            d.draw(r, &self.theme, area, self.scale);
        }
        if let Some(m) = self.menu.as_mut() {
            fade(r, "menu");
            m.draw(r, &self.theme, win_w, win_h, self.scale);
        }
        r.set_fade(1.0, 0.0);

        if rounded && r.transparent() {
            let (b, f) = (self.theme.background, self.theme.foreground);
            let edge = [b[0] + (f[0] - b[0]) * 0.22, b[1] + (f[1] - b[1]) * 0.22, b[2] + (f[2] - b[2]) * 0.22, 1.0];
            r.rounded_outline([0.0, 0.0, win_w, win_h], radius, self.scale.round().max(1.0), edge);
        }
        self.block_buttons = buttons;
    }

    fn running_visible(&self) -> bool {
        self.visible().iter().any(|(id, _)| {
            self.panes
                .get(id)
                .is_some_and(|p| p.mirror.blocks.iter().any(|b| b.state == zhell_proto::BlockState::Running))
        })
    }
}

impl ApplicationHandler<UserEvent> for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.gui.is_none()
            && let Err(e) = self.init(el)
        {
            self.error = Some(e);
            el.exit();
        }
    }

    fn user_event(&mut self, el: &ActiveEventLoop, event: UserEvent) {
        let Some(gui) = self.gui.as_mut() else { return };
        match event {
            UserEvent::Wake => {
                if !gui.pump() {
                    el.exit();
                }
            }
            UserEvent::ConfigChanged => gui.reload_config(),
            UserEvent::BackgroundChanged => gui.reload_background(),
            UserEvent::FontsReady => {
                let before = gui.renderer.cell_metrics();
                if gui.renderer.upgrade_fonts() {
                    if gui.renderer.cell_metrics() != before {
                        gui.relayout();
                    }
                    gui.window.request_redraw();
                }
            }
            UserEvent::BackgroundLoaded(l) => gui.background_loaded(*l),
        }
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        let Some(gui) = self.gui.as_mut() else { return };
        if gui.window_close_requested {
            gui.closing();
            el.exit();
            return;
        }
        gui.check_recorder();
        let now = Instant::now();
        let mut wake: Option<Instant> = None;
        if gui.cursor_blinks() {
            if now >= gui.blink_at {
                gui.blink_on = !gui.blink_on;
                gui.blink_at = now + BLINK_INTERVAL;
                gui.window.request_redraw();
            }
            wake = Some(gui.blink_at);
        }
        if gui.running_visible() {
            if now >= gui.tick_at {
                gui.tick_at = now + Duration::from_millis(250);
                gui.window.request_redraw();
            }
            wake = Some(wake.map_or(gui.tick_at, |w| w.min(gui.tick_at)));
        }

        let due: Vec<PaneId> = gui.pending_input.iter().filter(|(_, (_, t))| now >= *t).map(|(p, _)| *p).collect();
        for pane in due {
            if let Some((input, _)) = gui.pending_input.remove(&pane) {
                gui.host.send(ClientMsg::Input { pane, bytes: format!("{input}\r").into_bytes() });
            }
        }
        if let Some(t) = gui.pending_input.values().map(|(_, t)| *t).min() {
            wake = Some(wake.map_or(t, |w| w.min(t)));
        }

        if let Some((_, until, _)) = gui.toast {
            if now >= until {
                gui.toast = None;
                gui.window.request_redraw();
            } else {
                wake = Some(wake.map_or(until, |w| w.min(until)));
            }
        }
        if gui.background_animating() {
            if now >= gui.bg_frame_at {
                gui.bg_frame_at = now + Duration::from_millis(33);
                gui.window.request_redraw();
            }
            wake = Some(wake.map_or(gui.bg_frame_at, |w| w.min(gui.bg_frame_at)));
        }

        if gui.config.paste.auto_screen_share {
            wake = Some(wake.map_or(gui.next_recorder_check, |w| w.min(gui.next_recorder_check)));
        }
        el.set_control_flow(wake.map_or(ControlFlow::Wait, ControlFlow::WaitUntil));
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(gui) = self.gui.as_mut() else { return };
        match event {
            WindowEvent::CloseRequested => {
                if gui.request_close() {
                    gui.closing();
                    el.exit();
                }
            }
            WindowEvent::Resized(size) => {
                gui.renderer.resize(size.width, size.height);
                gui.relayout();
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                gui.scale = scale_factor as f32;
                gui.set_font_size(gui.font_size);
            }
            WindowEvent::RedrawRequested => gui.draw(),
            WindowEvent::ThemeChanged(_) => gui.follow_system_theme(),
            WindowEvent::Focused(f) => {
                gui.focused = f;
                if f {
                    gui.follow_system_theme();
                }
                if f {
                    gui.quake_had_focus = true;
                } else if gui.quake && gui.quake_had_focus && gui.config.quake.hide_on_unfocus && gui.dialog.is_none() {
                    gui.quake_had_focus = false;
                    gui.window.set_visible(false);
                }
                if let Some(pane) = gui.layout.focused() {
                    gui.host.send(ClientMsg::Focus { pane, focused: f });
                }
                gui.window.request_redraw();
            }
            WindowEvent::ModifiersChanged(m) => {
                gui.modifiers = m.state();
                gui.update_hover();
            }
            WindowEvent::KeyboardInput { event, is_synthetic: false, .. } => {
                if gui.preedit.is_some() {
                    return;
                }
                let pressed = event.state == ElementState::Pressed;
                if gui.menu.is_some() {
                    if pressed {
                        let res = gui.menu.as_mut().map(|m| m.key(&event.logical_key));
                        if let Some(res) = res {
                            gui.menu_result(res);
                        }
                    }
                    return;
                }
                if gui.findbar.is_some() && gui.dialog.is_none() && gui.palette.is_none() && gui.search.is_none() {
                    if pressed {
                        if gui.shortcut(&event) {
                            return;
                        }
                        let m = gui.modifiers;
                        let effect = gui.findbar.as_mut().map(|f| f.key(&event.logical_key, event.text.as_deref(), m));
                        if let Some(e) = effect {
                            gui.find_effect(e);
                        }
                    }
                    return;
                }
                if gui.copy_mode.is_some() && gui.dialog.is_none() && gui.findbar.is_none() {
                    if pressed {
                        let ctrl = gui.modifiers.control_key();
                        let effect = gui.copy_mode.as_mut().map(|c| c.key(&event.logical_key, event.text.as_deref(), ctrl));
                        gui.copy_mode_effect(effect);
                    }
                    return;
                }
                if gui.quick.is_some() && gui.dialog.is_none() {
                    if pressed {
                        let outcome = gui.quick.as_mut().map(|q| q.key(&event.logical_key, event.text.as_deref()));
                        gui.quick_select_outcome(outcome);
                    }
                    return;
                }
                if gui.dialog.is_some() {
                    if pressed {
                        let res = gui.dialog.as_mut().map(|d| d.key(&event.logical_key, event.text.as_deref()));
                        if let Some(res) = res {
                            gui.dialog_result(res);
                        }
                    }
                    return;
                }
                if gui.palette.is_some() {
                    if pressed {
                        let ctrl = gui.modifiers.control_key();
                        let effect = gui.palette.as_mut().map(|p| p.key(&event.logical_key, event.text.as_deref(), ctrl));
                        if let Some(e) = effect {
                            gui.palette_effect(e);
                        }
                        gui.window.request_redraw();
                    }
                    return;
                }
                if gui.search.is_some() {
                    if pressed {
                        let m = gui.modifiers;
                        let effect = gui.search.as_mut().map(|s| s.key(&event.logical_key, event.text.as_deref(), m));
                        if let Some(e) = effect {
                            gui.search_effect(e);
                        }
                    }
                    return;
                }
                log::debug!("key {:?} text {:?} mods {:?} pressed {pressed}", event.logical_key, event.text, gui.modifiers);
                if pressed {
                    gui.reset_blink();
                    if gui.shortcut(&event) {
                        return;
                    }
                }
                let modes = gui.focused_pane().map_or(0, |p| p.mirror.modes);
                let bytes = if modes & input::kitty::ANY != 0 {
                    use winit::platform::modifier_supplement::KeyEventExtModifierSupplement;
                    let base = event.key_without_modifiers();
                    let action = match (pressed, event.repeat) {
                        (false, _) => input::KeyAction::Release,
                        (true, true) => input::KeyAction::Repeat,
                        (true, false) => input::KeyAction::Press,
                    };
                    let k = input::KittyKey {
                        logical: &event.logical_key,
                        base: &base,
                        text: event.text.as_deref(),
                        mods: gui.modifiers,
                        action,
                    };
                    input::encode_kitty(&k, modes)
                } else if pressed {
                    input::encode_key(&event.logical_key, event.text.as_deref(), gui.modifiers, modes)
                } else {
                    None
                };
                if let Some(bytes) = bytes {
                    gui.write(bytes);
                }
            }
            WindowEvent::Ime(ime) => match ime {
                Ime::Preedit(text, _) => {
                    gui.preedit = (!text.is_empty()).then_some(text);
                    gui.window.request_redraw();
                }
                Ime::Commit(text) => {
                    gui.preedit = None;
                    gui.write(text.into_bytes());
                    gui.window.request_redraw();
                }
                Ime::Disabled => {
                    gui.preedit = None;
                    gui.window.request_redraw();
                }
                Ime::Enabled => {}
            },
            WindowEvent::CursorMoved { position, .. } => gui.pointer_moved(position),
            WindowEvent::MouseInput { state, button, .. } => {
                let pressed = state == ElementState::Pressed;
                match button {
                    winit::event::MouseButton::Left if pressed => gui.left_press(),
                    winit::event::MouseButton::Left => gui.left_release(),
                    winit::event::MouseButton::Middle if pressed => {
                        if let Some(titlebar::Hit::Tab(i) | titlebar::Hit::CloseTab(i)) = gui.title_hit(gui.cursor_pos) {
                            gui.close_tab(i);
                        } else if !gui.report_mouse(Some(MouseButton::Middle), true, false) {
                            gui.paste_primary();
                        }
                    }
                    winit::event::MouseButton::Middle => {
                        gui.report_mouse(Some(MouseButton::Middle), false, false);
                    }
                    winit::event::MouseButton::Right => {
                        if gui.menu.is_some() {
                            if pressed {
                                gui.menu = None;
                                gui.window.request_redraw();
                            }
                        } else if !gui.report_mouse(Some(MouseButton::Right), pressed, false) && pressed {
                            gui.open_menu();
                        }
                    }
                    _ => {}
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let lines = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y as f64 * 3.0,
                    MouseScrollDelta::PixelDelta(p) => p.y / gui.renderer.cell_metrics().height as f64,
                };
                gui.wheel(lines);
            }
            _ => {}
        }
    }
}

const HELP: &str = "\
zhell — a fast GPU terminal that remembers everything and never loses your sessions
by TheHolyOneZ · https://zsync.eu/zhell/

USAGE:
    zhell [PROGRAM [ARGS...]]    open a window (PROGRAM runs in the first new tab)

OPTIONS:
    -h, --help       this help
    -V, --version    print the version
    --new-window     open a window with a fresh shell, leaving background sessions alone
    --quake          show or hide the drop-down window (bind it to a global shortcut)
    --import-theme FILE [NAME]
                     convert an iTerm2 (.itermcolors), Windows Terminal (.json)
                     or Alacritty (.toml) colour scheme into a Zhell theme

FILES:
    config     ~/.config/zhell/zhell.toml   (ZHELL_CONFIG overrides; reloaded live)
    history    ~/.local/state/zhell/history.db
    sessions   kept by zhelld; `zhelld --stop` ends them all

KEYS (defaults, change them under [keys]):
    ctrl+shift+p  command palette     ctrl+shift+f  search history
    ctrl+shift+t  new tab             ctrl+shift+d/e  split right/down
    ctrl+shift+w  close pane          alt+arrows    move between panes
    ctrl+up/down  jump between commands
    ctrl+shift+k  copy mode           ctrl+shift+j  quick select

More: https://zsync.eu/zhell/ · all projects: https://zsync.eu
";

fn export_path(cmd: &str, ext: &str) -> PathBuf {
    let slug: String = cmd.chars().map(|c| if c.is_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).collect();
    let slug = slug.split('-').filter(|p| !p.is_empty()).take(5).collect::<Vec<_>>().join("-");
    let stamp = jiff::Zoned::now().strftime("%Y%m%d-%H%M%S").to_string();
    let dir = dirs::download_dir().or_else(dirs::home_dir).unwrap_or_else(|| PathBuf::from("."));
    dir.join(format!("zhell-{}-{stamp}.{ext}", if slug.is_empty() { "output" } else { &slug }))
}

fn recording_path(label: &str) -> PathBuf {
    let dir = dirs::video_dir().or_else(dirs::home_dir).unwrap_or_default().join("Zhell");
    let stamp = jiff::Zoned::now().strftime("%Y-%m-%d-%H%M").to_string();
    let slug: String = label.chars().map(|c| if c.is_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).collect();
    let slug = slug.split('-').filter(|s| !s.is_empty()).take(4).collect::<Vec<_>>().join("-");
    let name = format!("zhell-{stamp}{}{slug}.cast", if slug.is_empty() { "" } else { "-" });
    let path = dir.join(&name);

    (1..)
        .map(|n| if n == 1 { path.clone() } else { dir.join(name.replace(".cast", &format!("-{n}.cast"))) })
        .find(|p| !p.exists())
        .unwrap_or(path)
}

fn quake_geometry(m: &winit::monitor::MonitorHandle, q: &zhell_core::config::QuakeConfig) -> (PhysicalPosition<i32>, PhysicalSize<u32>) {
    let (mp, ms) = (m.position(), m.size());
    let w = (ms.width as f32 * q.width).round() as u32;
    let h = (ms.height as f32 * q.height).round() as u32;
    (PhysicalPosition::new(mp.x + (ms.width.saturating_sub(w) / 2) as i32, mp.y), PhysicalSize::new(w, h))
}

fn toggle_existing_quake() -> bool {
    let Ok(host) = IpcHost::connect(&zhell_daemon::ipc::socket_name()) else { return false };
    host.send(ClientMsg::Hello { proto_version: PROTO_VERSION, client_name: "zhell-toggle".into(), restore: false });
    host.send(ClientMsg::ToggleQuake);
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        match host.try_recv() {
            Some(ServerMsg::QuakeHandled { handled }) => return handled,
            Some(ServerMsg::VersionMismatch { .. }) => return false,
            Some(_) => {}
            None => std::thread::sleep(Duration::from_millis(5)),
        }
    }
    false
}

#[derive(Debug, Default, PartialEq)]
struct Cli {
    new_window: bool,

    quake: bool,
    attach: Option<PaneId>,
    program: Option<(String, Vec<String>)>,
}

impl Cli {
    fn parse(args: impl IntoIterator<Item = String>) -> Self {
        let mut cli = Cli::default();
        let mut args = args.into_iter().peekable();
        while let Some(a) = args.peek() {
            match a.as_str() {
                "--new-window" => cli.new_window = true,
                "--quake" => cli.quake = true,
                "--attach" => {
                    args.next();
                    cli.attach = args.peek().and_then(|n| n.parse().ok()).map(PaneId);
                }
                "--" => {
                    args.next();
                    break;
                }
                _ => break,
            }
            args.next();
        }
        let rest: Vec<String> = args.collect();
        cli.program = rest.split_first().map(|(p, r)| (p.clone(), r.to_vec()));
        cli
    }
}

static STARTED: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();

fn startup_mark(stage: &str) {
    if let Some(t) = STARTED.get() {
        log::debug!("startup: {stage} at {} ms", t.elapsed().as_millis());
    }
}

fn main() -> anyhow::Result<()> {
    STARTED.get_or_init(Instant::now);
    match std::env::args().nth(1).as_deref() {
        Some("-h" | "--help") => {
            print!("{HELP}");
            return Ok(());
        }
        Some("-V" | "--version") => {
            println!("zhell {} — by TheHolyOneZ · {}", env!("CARGO_PKG_VERSION"), env!("CARGO_PKG_HOMEPAGE"));
            return Ok(());
        }
        Some("--import-theme") => {
            let args: Vec<String> = std::env::args().skip(2).collect();
            let Some(src) = args.first() else {
                anyhow::bail!("usage: zhell --import-theme FILE [NAME]");
            };
            let (name, path) = zhell_core::themes::import(Path::new(src), args.get(1).map(String::as_str))?;
            println!("Imported to {}\nUse it with  theme = \"{name}\"  in zhell.toml, or pick it in the command palette.", path.display());
            return Ok(());
        }
        _ => {}
    }
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();

    if Cli::parse(std::env::args().skip(1)).quake && toggle_existing_quake() {
        return Ok(());
    }
    let event_loop = EventLoop::<UserEvent>::with_user_event().build()?;

    let family = config::default_path().and_then(|p| Config::load(&p).ok()).map(|c| c.font.family).filter(|f| !f.is_empty());
    let proxy = event_loop.create_proxy();
    let cache = dirs::cache_dir().map(|d| d.join("zhell").join("font-quickstart"));
    zhell_render::preload_fonts(family, cache, Some(Box::new(move || drop(proxy.send_event(UserEvent::FontsReady)))));
    let mut app = App { proxy: event_loop.create_proxy(), gui: None, error: None };
    event_loop.run_app(&mut app)?;
    match app.error {
        Some(e) => Err(e),
        None => Ok(()),
    }
}
