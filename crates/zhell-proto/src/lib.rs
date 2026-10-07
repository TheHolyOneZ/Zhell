use serde::{Deserialize, Serialize};

pub const PROTO_VERSION: u32 = 18;

pub const MAX_FRAME_LEN: u32 = 64 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PaneId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TermSize {
    pub cols: u16,
    pub rows: u16,

    pub cell_width: u16,
    pub cell_height: u16,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpawnSpec {
    pub program: Option<String>,
    pub args: Vec<String>,
    pub cwd: Option<String>,
    pub env: Vec<(String, String)>,

    pub integration: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClientMsg {
    Hello { proto_version: u32, client_name: String, restore: bool },

    CreatePane { req: u32, spawn: SpawnSpec, size: TermSize },
    Input { pane: PaneId, bytes: Vec<u8> },
    Resize { pane: PaneId, size: TermSize },

    Scroll { pane: PaneId, delta: i32 },
    ScrollToBottom { pane: PaneId },

    Ack { pane: PaneId, seq: u64 },
    Focus { pane: PaneId, focused: bool },

    SetOptions(HostOptions),

    SelectStart { pane: PaneId, row: u16, col: u16, right_half: bool, kind: SelectKind },

    SelectUpdate { pane: PaneId, row: i32, col: u16, right_half: bool },
    SelectClear { pane: PaneId },

    ResolvePath { pane: PaneId, req: u32, path: String },

    Find { pane: PaneId, query: String, regex: bool },

    FindNext { pane: PaneId, older: bool },
    FindClose { pane: PaneId },

    Fold { pane: PaneId, block: u32, folded: bool },

    CopyBlockOutput { pane: PaneId, block: u32, target: CopyTarget },

    JumpBlock { pane: PaneId, forward: bool },

    QueryForeground { req: u32, pane: PaneId },

    StopPort { pane: PaneId, port: u16 },

    HistorySearch { req: u32, query: HistoryQuery },

    HistoryGet { req: u32, id: i64 },
    HistoryStar { id: i64, starred: bool },
    HistoryNote { id: i64, note: Option<String> },
    HistoryTemplate { id: i64, template: Option<String> },

    HistoryForget { id: i64 },

    Copy { pane: PaneId, target: CopyTarget },
    ClosePane { pane: PaneId },

    ToggleQuake,

    CopyMode { pane: PaneId, cmd: CopyCmd },

    BlockCells { req: u32, pane: PaneId, block: u32 },

    Record { pane: PaneId, path: Option<String> },

    Attach { pane: PaneId },

    StoreLayout(Vec<u8>),

    Bye,

    Shutdown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CopyCmd {
    Enter,
    Exit,
    Up,
    Down,
    Left,
    Right,
    LineStart,
    LineEnd,
    FirstNonBlank,
    WordNext,
    WordPrev,
    WordEnd,
    BigWordNext,
    BigWordPrev,
    BigWordEnd,
    ScreenTop,
    ScreenMiddle,
    ScreenBottom,
    ParagraphUp,
    ParagraphDown,
    Bracket,

    Top,
    Bottom,
    HalfPageUp,
    HalfPageDown,
    PageUp,
    PageDown,

    PrevPrompt,
    NextPrompt,

    Select(SelectKind),

    Yank,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryQuery {
    pub text: String,
    pub failed_only: bool,
    pub starred_only: bool,

    pub cwd_prefix: Option<String>,

    pub since_ms: Option<u64>,
    pub until_ms: Option<u64>,
    pub limit: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryHit {
    pub id: i64,
    pub cmd: String,
    pub cwd: Option<String>,
    pub exit_code: Option<i32>,
    pub started_ms: u64,
    pub duration_ms: u64,

    pub snippet: String,
    pub starred: bool,
    pub note: Option<String>,

    pub template: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Restore {
    pub layout: Vec<u8>,
    pub panes: Vec<PaneId>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ServerMsg {
    Detached { pane: PaneId },

    RemoteCwd { pane: PaneId, host: String, cwd: String },

    Ready { pane: PaneId },

    Recording { pane: PaneId, path: Option<String>, done: bool, error: Option<String> },
    BlockCells { req: u32, rows: Vec<Vec<Cell>> },

    Quake,

    QuakeHandled { handled: bool },
    HelloOk { proto_version: u32, host_version: String, restore: Option<Restore> },
    VersionMismatch { host_proto_version: u32 },
    PaneCreated { req: u32, pane: PaneId },
    SpawnFailed { req: u32, error: String },
    Frame(FrameDiff),
    Title { pane: PaneId, title: Option<String> },
    Cwd { pane: PaneId, cwd: String },
    Mark { pane: PaneId, mark: ShellMark },
    Bell { pane: PaneId },
    Clipboard { pane: PaneId, text: String },
    CopyText { pane: PaneId, text: String, target: CopyTarget },
    Foreground { req: u32, name: Option<String> },

    Image { pane: PaneId, id: u32, width: u32, height: u32, cols: u16, rows: u16, rgba: Vec<u8> },

    Ports { pane: PaneId, ports: Vec<u16> },

    Background { count: u32 },
    HistoryResults { req: u32, hits: Vec<HistoryHit> },

    HistoryEntry { req: u32, entry: Option<(HistoryHit, String)> },

    PathResolved { req: u32, path: Option<String> },
    Exited { pane: PaneId, code: Option<i32> },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShellMark {
    pub kind: MarkKind,
    pub abs_line: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MarkKind {
    PromptStart,
    CommandStart,
    OutputStart,
    CommandFinished { exit_code: Option<i32> },
    CommandText(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostOptions {
    pub scrollback_lines: u32,

    pub cursor_shape: CursorShape,
    pub cursor_blink: bool,
}

impl Default for HostOptions {
    fn default() -> Self {
        Self { scrollback_lines: 100_000, cursor_shape: CursorShape::Block, cursor_blink: false }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SelectKind {
    Simple,

    Block,

    Word,

    Line,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CopyTarget {
    Clipboard,

    File,

    Primary,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectionSpan {
    pub start_row: i32,
    pub start_col: u16,
    pub end_row: i32,
    pub end_col: u16,
    pub block: bool,
}

impl SelectionSpan {
    pub fn contains(&self, row: i32, col: u16) -> bool {
        if row < self.start_row || row > self.end_row {
            return false;
        }
        if self.block {
            return col >= self.start_col && col <= self.end_col;
        }
        (row != self.start_row || col >= self.start_col) && (row != self.end_row || col <= self.end_col)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockSpan {
    pub id: u32,

    pub prompt_row: i32,

    pub output_row: i32,
    pub cmd: Option<String>,
    pub state: BlockState,

    pub started_ms: Option<u64>,
    pub duration_ms: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FoldRow {
    pub row: u16,
    pub block: u32,
    pub hidden: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FindState {
    pub matches: Vec<SelectionSpan>,

    pub current: Option<SelectionSpan>,

    pub found: bool,

    pub invalid: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageRow {
    pub row: u16,
    pub col: u16,
    pub id: u32,
    pub image_row: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BlockState {
    Editing,
    Running,
    Done { exit: Option<i32> },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FrameDiff {
    pub pane: PaneId,
    pub seq: u64,
    pub cols: u16,
    pub rows: u16,
    pub history_len: u32,
    pub display_offset: u32,
    pub cursor: CursorState,
    pub modes: u32,

    pub selection: Option<SelectionSpan>,

    pub blocks: Vec<BlockSpan>,

    pub images: Vec<ImageRow>,

    pub find: Option<FindState>,

    pub folds: Vec<FoldRow>,

    pub full: bool,
    pub lines: Vec<LineUpdate>,
}

pub mod mode {
    pub const SHOW_CURSOR: u32 = 1;
    pub const APP_CURSOR: u32 = 1 << 1;
    pub const APP_KEYPAD: u32 = 1 << 2;
    pub const MOUSE_REPORT_CLICK: u32 = 1 << 3;
    pub const BRACKETED_PASTE: u32 = 1 << 4;
    pub const SGR_MOUSE: u32 = 1 << 5;
    pub const MOUSE_MOTION: u32 = 1 << 6;
    pub const FOCUS_IN_OUT: u32 = 1 << 11;
    pub const ALT_SCREEN: u32 = 1 << 12;
    pub const MOUSE_DRAG: u32 = 1 << 13;
    pub const UTF8_MOUSE: u32 = 1 << 14;
    pub const ALTERNATE_SCROLL: u32 = 1 << 15;
    pub const KITTY_KEYBOARD: u32 = 0b1_1111 << 18;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CursorState {
    pub row: u16,
    pub col: u16,
    pub shape: CursorShape,
    pub blinking: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CursorShape {
    Block,
    Underline,
    Beam,
    HollowBlock,
    Hidden,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineUpdate {
    pub row: u16,
    pub cells: Vec<Cell>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cell {
    pub ch: char,

    pub zerowidth: Vec<char>,
    pub fg: Color,
    pub bg: Color,
    pub underline_color: Option<Color>,
    pub flags: u16,
    pub hyperlink: Option<String>,
}

impl Default for Cell {
    fn default() -> Self {
        Self {
            ch: ' ',
            zerowidth: Vec::new(),
            fg: Color::Named(named::FOREGROUND),
            bg: Color::Named(named::BACKGROUND),
            underline_color: None,
            flags: 0,
            hyperlink: None,
        }
    }
}

pub mod flags {
    pub const INVERSE: u16 = 1;
    pub const BOLD: u16 = 1 << 1;
    pub const ITALIC: u16 = 1 << 2;
    pub const UNDERLINE: u16 = 1 << 3;
    pub const WRAPLINE: u16 = 1 << 4;
    pub const WIDE_CHAR: u16 = 1 << 5;
    pub const WIDE_CHAR_SPACER: u16 = 1 << 6;
    pub const DIM: u16 = 1 << 7;
    pub const HIDDEN: u16 = 1 << 8;
    pub const STRIKEOUT: u16 = 1 << 9;
    pub const LEADING_WIDE_CHAR_SPACER: u16 = 1 << 10;
    pub const DOUBLE_UNDERLINE: u16 = 1 << 11;
    pub const UNDERCURL: u16 = 1 << 12;
    pub const DOTTED_UNDERLINE: u16 = 1 << 13;
    pub const DASHED_UNDERLINE: u16 = 1 << 14;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Color {
    Named(u16),

    Indexed(u8),
    Rgb(u8, u8, u8),
}

pub mod named {
    pub const FOREGROUND: u16 = 256;
    pub const BACKGROUND: u16 = 257;
    pub const CURSOR: u16 = 258;
}

#[derive(Debug, thiserror::Error)]
pub enum CodecError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("encode: {0}")]
    Encode(#[from] bincode::error::EncodeError),
    #[error("decode: {0}")]
    Decode(#[from] bincode::error::DecodeError),
    #[error("frame of {0} bytes exceeds the limit")]
    TooLarge(u32),
}

fn config() -> impl bincode::config::Config {
    bincode::config::standard()
}

pub fn write_frame<W: std::io::Write, M: Serialize>(w: &mut W, msg: &M) -> Result<(), CodecError> {
    let body = bincode::serde::encode_to_vec(msg, config())?;
    let len = u32::try_from(body.len()).map_err(|_| CodecError::TooLarge(u32::MAX))?;
    if len > MAX_FRAME_LEN {
        return Err(CodecError::TooLarge(len));
    }
    w.write_all(&len.to_le_bytes())?;
    w.write_all(&body)?;
    Ok(())
}

pub fn read_frame<R: std::io::Read, M: for<'de> Deserialize<'de>>(
    r: &mut R,
) -> Result<Option<M>, CodecError> {
    let mut len = [0u8; 4];
    match r.read_exact(&mut len) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e.into()),
    }
    let len = u32::from_le_bytes(len);
    if len > MAX_FRAME_LEN {
        return Err(CodecError::TooLarge(len));
    }
    let mut body = vec![0u8; len as usize];
    r.read_exact(&mut body)?;
    let (msg, _) = bincode::serde::decode_from_slice(&body, config())?;
    Ok(Some(msg))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_roundtrip() {
        let msgs = vec![
            ClientMsg::Hello { proto_version: PROTO_VERSION, client_name: "test".into(), restore: true },
            ClientMsg::Input { pane: PaneId(3), bytes: b"ls\r".to_vec() },
        ];
        let mut buf = Vec::new();
        for m in &msgs {
            write_frame(&mut buf, m).unwrap();
        }
        let mut r = buf.as_slice();
        let mut out = Vec::new();
        while let Some(m) = read_frame::<_, ClientMsg>(&mut r).unwrap() {
            out.push(m);
        }
        assert_eq!(out, msgs);
    }

    #[test]
    fn selection_contains() {
        let s = SelectionSpan { start_row: 1, start_col: 5, end_row: 3, end_col: 2, block: false };
        assert!(!s.contains(1, 4));
        assert!(s.contains(1, 5) && s.contains(2, 0) && s.contains(2, 99) && s.contains(3, 2));
        assert!(!s.contains(3, 3) && !s.contains(0, 6) && !s.contains(4, 0));
        let b = SelectionSpan { block: true, start_col: 2, end_col: 5, ..s };
        assert!(b.contains(2, 3) && !b.contains(2, 6) && !b.contains(1, 1));
    }

    #[test]
    fn rejects_oversized_frame() {
        let buf = (MAX_FRAME_LEN + 1).to_le_bytes();
        let err = read_frame::<_, ClientMsg>(&mut buf.as_slice()).unwrap_err();
        assert!(matches!(err, CodecError::TooLarge(_)));
    }
}
