use std::collections::BTreeMap;
use std::fmt;

use serde::Deserialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Copy,
    Paste,
    NewTab,
    ClosePane,
    SplitRight,
    SplitDown,
    ToggleZoom,
    NextTab,
    PrevTab,
    SelectTab1,
    SelectTab2,
    SelectTab3,
    SelectTab4,
    SelectTab5,
    SelectTab6,
    SelectTab7,
    SelectTab8,
    SelectTab9,
    FocusLeft,
    FocusRight,
    FocusUp,
    FocusDown,
    FontBigger,
    FontSmaller,
    FontReset,
    ScrollPageUp,
    ScrollPageDown,
    ScrollTop,
    ScrollBottom,

    PrevBlock,
    NextBlock,

    SearchHistory,

    CommandPalette,

    ToggleScreenShare,

    OpenWorkspace,

    Find,

    OpenSettings,

    RenameTab,

    NewWindow,

    MoveToNewWindow,

    InstallRemoteIntegration,

    ToggleBroadcast,

    ToggleRecording,

    QuickSelect,

    CopyMode,

    About,

    OpacityDown,
    OpacityUp,
    OpacityReset,

    ResizeLeft,
    ResizeRight,
    ResizeUp,
    ResizeDown,

    None,
}

impl Action {
    pub const PALETTE: &[(Action, &str)] = &[
        (Action::NewTab, "New tab"),
        (Action::NewWindow, "New window"),
        (Action::ToggleBroadcast, "Broadcast input to all panes in this tab (toggle)"),
        (Action::ToggleRecording, "Record this pane (asciinema .cast, toggle)"),
        (Action::QuickSelect, "Quick select: copy a URL, path or hash by its label"),
        (Action::CopyMode, "Copy mode: select with the keyboard (vi keys)"),
        (Action::OpacityDown, "Opacity: more see-through"),
        (Action::OpacityUp, "Opacity: more solid"),
        (Action::OpacityReset, "Opacity: back to the configured value"),
        (Action::ResizeLeft, "Resize pane: border left"),
        (Action::ResizeRight, "Resize pane: border right"),
        (Action::ResizeUp, "Resize pane: border up"),
        (Action::ResizeDown, "Resize pane: border down"),
        (Action::MoveToNewWindow, "Move pane to new window"),
        (Action::SplitRight, "Split pane right"),
        (Action::SplitDown, "Split pane down"),
        (Action::ClosePane, "Close pane"),
        (Action::ToggleZoom, "Zoom pane (toggle)"),
        (Action::NextTab, "Next tab"),
        (Action::PrevTab, "Previous tab"),
        (Action::FocusLeft, "Focus pane left"),
        (Action::FocusRight, "Focus pane right"),
        (Action::FocusUp, "Focus pane up"),
        (Action::FocusDown, "Focus pane down"),
        (Action::Find, "Find in scrollback"),
        (Action::OpenSettings, "Open settings file"),
        (Action::RenameTab, "Rename tab"),
        (Action::SearchHistory, "Search history"),
        (Action::PrevBlock, "Jump to previous command"),
        (Action::NextBlock, "Jump to next command"),
        (Action::Copy, "Copy selection"),
        (Action::Paste, "Paste"),
        (Action::FontBigger, "Font: bigger"),
        (Action::FontSmaller, "Font: smaller"),
        (Action::FontReset, "Font: reset size"),
        (Action::ScrollTop, "Scroll to top"),
        (Action::ScrollBottom, "Scroll to bottom"),
        (Action::ToggleScreenShare, "Screen-share mode (hide secrets)"),
        (Action::OpenWorkspace, "Project: open workspace"),
        (Action::About, "About Zhell"),
        (Action::InstallRemoteIntegration, "SSH: install shell integration on this host"),
    ];

    pub fn select_tab_index(self) -> Option<usize> {
        use Action::*;
        let all = [SelectTab1, SelectTab2, SelectTab3, SelectTab4, SelectTab5, SelectTab6, SelectTab7, SelectTab8, SelectTab9];
        all.iter().position(|a| *a == self)
    }
}

pub const CTRL: u8 = 1;
pub const SHIFT: u8 = 2;
pub const ALT: u8 = 4;
pub const SUPER: u8 = 8;

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct KeyCombo {
    pub mods: u8,
    pub key: String,
}

const ALIASES: &[(&str, &str)] = &[
    ("plus", "+"),
    ("minus", "-"),
    ("equal", "="),
    ("comma", ","),
    ("period", "."),
    ("slash", "/"),
    ("backslash", "\\"),
    ("semicolon", ";"),
    ("space", " "),
    ("return", "enter"),
    ("esc", "escape"),
    ("pgup", "pageup"),
    ("pgdn", "pagedown"),
    ("arrowleft", "left"),
    ("arrowright", "right"),
    ("arrowup", "up"),
    ("arrowdown", "down"),
];

impl KeyCombo {
    pub fn new(mods: u8, key: &str) -> Self {
        let key = key.to_lowercase();
        let key = ALIASES.iter().find(|(a, _)| *a == key).map_or(key, |(_, k)| (*k).to_owned());
        Self { mods, key }
    }

    pub fn parse(spec: &str) -> Result<Self, String> {
        let spec = spec.trim().to_lowercase();

        let (mods_part, key) = match spec.strip_suffix("++") {
            Some(m) => (format!("{m}+"), "+".to_owned()),
            None => match spec.rsplit_once('+') {
                Some((m, k)) => (format!("{m}+"), k.to_owned()),
                None => (String::new(), spec.clone()),
            },
        };
        if key.is_empty() {
            return Err(format!("{spec:?}: missing key"));
        }
        let mut mods = 0;
        for m in mods_part.split('+').filter(|m| !m.is_empty()) {
            mods |= match m {
                "ctrl" | "control" => CTRL,
                "shift" => SHIFT,
                "alt" | "option" => ALT,
                "super" | "cmd" | "win" | "meta" => SUPER,
                other => return Err(format!("{spec:?}: unknown modifier {other:?}")),
            };
        }
        Ok(Self::new(mods, &key))
    }
}

impl fmt::Display for KeyCombo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (bit, name) in [(CTRL, "ctrl"), (SHIFT, "shift"), (ALT, "alt"), (SUPER, "super")] {
            if self.mods & bit != 0 {
                write!(f, "{name}+")?;
            }
        }
        f.write_str(&self.key)
    }
}

pub const DEFAULT_BINDINGS: &[(&str, Action)] = &[
    ("ctrl+shift+c", Action::Copy),
    ("ctrl+insert", Action::Copy),
    ("ctrl+shift+v", Action::Paste),
    ("shift+insert", Action::Paste),
    ("ctrl+shift+t", Action::NewTab),
    ("ctrl+shift+n", Action::NewWindow),
    ("ctrl+shift+b", Action::ToggleBroadcast),
    ("ctrl+shift+r", Action::ToggleRecording),
    ("ctrl+shift+space", Action::QuickSelect),

    ("ctrl+shift+j", Action::QuickSelect),
    ("ctrl+shift+x", Action::CopyMode),

    ("ctrl+shift+k", Action::CopyMode),
    ("ctrl+shift+alt+left", Action::ResizeLeft),
    ("ctrl+shift+alt+right", Action::ResizeRight),
    ("ctrl+shift+alt+up", Action::ResizeUp),
    ("ctrl+shift+alt+down", Action::ResizeDown),
    ("ctrl+shift+w", Action::ClosePane),
    ("ctrl+shift+d", Action::SplitRight),
    ("ctrl+shift+e", Action::SplitDown),
    ("ctrl+shift+z", Action::ToggleZoom),
    ("ctrl+tab", Action::NextTab),
    ("ctrl+shift+tab", Action::PrevTab),
    ("ctrl+pagedown", Action::NextTab),
    ("ctrl+pageup", Action::PrevTab),
    ("alt+1", Action::SelectTab1),
    ("alt+2", Action::SelectTab2),
    ("alt+3", Action::SelectTab3),
    ("alt+4", Action::SelectTab4),
    ("alt+5", Action::SelectTab5),
    ("alt+6", Action::SelectTab6),
    ("alt+7", Action::SelectTab7),
    ("alt+8", Action::SelectTab8),
    ("alt+9", Action::SelectTab9),
    ("alt+left", Action::FocusLeft),
    ("alt+right", Action::FocusRight),
    ("alt+up", Action::FocusUp),
    ("alt+down", Action::FocusDown),
    ("ctrl+equal", Action::FontBigger),
    ("ctrl+plus", Action::FontBigger),
    ("ctrl+shift+plus", Action::FontBigger),
    ("ctrl+minus", Action::FontSmaller),
    ("ctrl+0", Action::FontReset),
    ("shift+pageup", Action::ScrollPageUp),
    ("shift+pagedown", Action::ScrollPageDown),
    ("shift+home", Action::ScrollTop),
    ("shift+end", Action::ScrollBottom),
    ("ctrl+up", Action::PrevBlock),
    ("ctrl+down", Action::NextBlock),
    ("ctrl+shift+f", Action::SearchHistory),
    ("ctrl+shift+p", Action::CommandPalette),
    ("ctrl+shift+h", Action::ToggleScreenShare),
    ("ctrl+shift+o", Action::OpenWorkspace),
    ("ctrl+shift+g", Action::Find),
    ("ctrl+comma", Action::OpenSettings),
];

#[derive(Clone, Debug, PartialEq)]
pub struct Keymap(BTreeMap<KeyCombo, Action>);

impl Default for Keymap {
    fn default() -> Self {
        Self(
            DEFAULT_BINDINGS
                .iter()
                .map(|(k, a)| (KeyCombo::parse(k).expect("valid default binding"), *a))
                .collect(),
        )
    }
}

impl Keymap {
    pub fn with_overrides(user: &BTreeMap<String, Action>) -> Result<Self, String> {
        let mut map = Self::default();
        for (spec, action) in user {
            let combo = KeyCombo::parse(spec)?;
            if *action == Action::None {
                map.0.remove(&combo);
            } else {
                map.0.insert(combo, *action);
            }
        }
        Ok(map)
    }

    pub fn get(&self, combo: &KeyCombo) -> Option<Action> {
        self.0.get(combo).copied()
    }

    pub fn key_for(&self, action: Action) -> Option<&KeyCombo> {
        self.0
            .iter()
            .filter(|(_, a)| **a == action)
            .map(|(k, _)| k)
            .max_by_key(|k| (k.key.chars().count() == 1, k.mods.count_ones()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_specs() {
        assert_eq!(KeyCombo::parse("Ctrl+Shift+T").unwrap(), KeyCombo::new(CTRL | SHIFT, "t"));
        assert_eq!(KeyCombo::parse("ctrl++").unwrap(), KeyCombo::new(CTRL, "+"));
        assert_eq!(KeyCombo::parse("ctrl+plus").unwrap(), KeyCombo::new(CTRL, "+"));
        assert_eq!(KeyCombo::parse("alt+ArrowLeft").unwrap(), KeyCombo::new(ALT, "left"));
        assert_eq!(KeyCombo::parse("f5").unwrap(), KeyCombo::new(0, "f5"));
        assert!(KeyCombo::parse("hyper+x").is_err());
        assert!(KeyCombo::parse("ctrl+").is_err());
    }

    #[test]
    fn overrides_and_unbinding() {
        let mut user = BTreeMap::new();
        user.insert("alt+t".to_owned(), Action::NewTab);
        user.insert("ctrl+shift+w".to_owned(), Action::None);
        let k = Keymap::with_overrides(&user).unwrap();
        assert_eq!(k.get(&KeyCombo::new(ALT, "t")), Some(Action::NewTab));
        assert_eq!(k.get(&KeyCombo::new(CTRL | SHIFT, "t")), Some(Action::NewTab));
        assert_eq!(k.get(&KeyCombo::new(CTRL | SHIFT, "w")), None);
    }

    #[test]
    fn key_for_prefers_familiar_binding() {
        let k = Keymap::default();
        assert_eq!(k.key_for(Action::Paste).unwrap().to_string(), "ctrl+shift+v");
        assert_eq!(k.key_for(Action::Copy).unwrap().to_string(), "ctrl+shift+c");
    }

    #[test]
    fn display_roundtrip() {
        let c = KeyCombo::parse("shift+ctrl+pgup").unwrap();
        assert_eq!(c.to_string(), "ctrl+shift+pageup");
        assert_eq!(KeyCombo::parse(&c.to_string()).unwrap(), c);
    }
}
