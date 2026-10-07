use std::io::{Read, Write};
use std::sync::Arc;
use std::thread;

use alacritty_terminal::event::{Event as TermEvent, EventListener, WindowSize};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::cell::Cell as TermCell;
use alacritty_terminal::term::{Config, Term, TermDamage};
use alacritty_terminal::vte::ansi::{self, Processor};
use crossbeam_channel::Sender;
use parking_lot::Mutex;
use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};
use zhell_core::prescan::{OscEvent, OscPrescanner};
use zhell_proto::{
    Cell, Color, CursorShape, CursorState, FrameDiff, HostOptions, LineUpdate, MarkKind, PaneId, SelectKind,
    SelectionSpan, ServerMsg, ShellMark, SpawnSpec, TermSize,
};

use crate::{Event, Sink};

pub(crate) struct GridSize {
    cols: usize,
    rows: usize,
}

impl Dimensions for GridSize {
    fn total_lines(&self) -> usize {
        self.rows
    }
    fn screen_lines(&self) -> usize {
        self.rows
    }
    fn columns(&self) -> usize {
        self.cols
    }
}

type PtyWriter = Arc<Mutex<Box<dyn Write + Send>>>;

#[derive(Clone)]
pub struct Listener {
    pane: PaneId,
    out: Sink,
    writer: PtyWriter,
    size: Arc<Mutex<TermSize>>,
    title: Arc<Mutex<Option<String>>>,
}

impl Listener {
    fn write_pty(&self, s: &str) {
        let mut w = self.writer.lock();
        let _ = w.write_all(s.as_bytes()).and_then(|()| w.flush());
    }
}

impl EventListener for Listener {
    fn send_event(&self, event: TermEvent) {
        match event {
            TermEvent::Title(t) => {
                *self.title.lock() = Some(t.clone());
                self.out.send(ServerMsg::Title { pane: self.pane, title: Some(t) })
            }
            TermEvent::ResetTitle => {
                *self.title.lock() = None;
                self.out.send(ServerMsg::Title { pane: self.pane, title: None })
            }
            TermEvent::Bell => self.out.send(ServerMsg::Bell { pane: self.pane }),
            TermEvent::ClipboardStore(_, text) => {
                self.out.send(ServerMsg::Clipboard { pane: self.pane, text })
            }
            TermEvent::PtyWrite(s) => self.write_pty(&s),
            TermEvent::TextAreaSizeRequest(fmt) => {
                let s = *self.size.lock();
                self.write_pty(&fmt(WindowSize {
                    num_lines: s.rows,
                    num_cols: s.cols,
                    cell_width: s.cell_width,
                    cell_height: s.cell_height,
                }));
            }

            TermEvent::ColorRequest(..)
            | TermEvent::ClipboardLoad(..)
            | TermEvent::MouseCursorDirty
            | TermEvent::CursorBlinkingChange
            | TermEvent::Wakeup
            | TermEvent::Exit
            | TermEvent::ChildExit(_) => {}
        }
    }
}

#[derive(Clone)]
pub struct Recorder {
    pub tx: Sender<crate::history::HistoryCmd>,
    pub settings: Arc<crate::history::HistorySettings>,
    pub host: Arc<String>,
}

pub struct PaneCtx {
    pub options: HostOptions,
    pub sink: Sink,
    pub events: Sender<Event>,
    pub recorder: Option<Recorder>,
}

struct ReaderState {
    id: PaneId,
    writer: PtyWriter,
    size: Arc<Mutex<TermSize>>,
    images: Arc<Mutex<crate::images::Images>>,
    term: Arc<FairMutex<Term<Listener>>>,
    cwd: Arc<Mutex<Option<String>>>,
    blocks: Arc<Mutex<crate::blocks::Blocks>>,
    out: Sink,
    events: Sender<Event>,
    recorder: Option<Recorder>,
    cast: Arc<Mutex<Option<crate::cast::Cast>>>,
}

pub struct Find {
    regex: Option<alacritty_terminal::term::search::RegexSearch>,
    current: Option<alacritty_terminal::term::search::Match>,
}

pub struct Pane {
    pub id: PaneId,

    pub sink: Sink,
    pub title: Arc<Mutex<Option<String>>>,
    pub blocks: Arc<Mutex<crate::blocks::Blocks>>,

    find: Option<Find>,

    folds: crate::folds::Folds,

    view: Option<View>,

    force_full: bool,
    pub images: Arc<Mutex<crate::images::Images>>,

    cast: Arc<Mutex<Option<crate::cast::Cast>>>,
    term: Arc<FairMutex<Term<Listener>>>,
    master: Box<dyn MasterPty + Send>,
    writer: PtyWriter,
    killer: Box<dyn ChildKiller + Send + Sync>,
    size: Arc<Mutex<TermSize>>,

    pub shell_pid: Option<u32>,

    pub ports: std::collections::BTreeMap<u16, u32>,

    cwd: Arc<Mutex<Option<String>>>,

    start_dir: Option<String>,

    pub inflight: Option<u64>,

    pub dirty: bool,
    next_seq: u64,
}

impl Pane {
    pub fn spawn(
        id: PaneId,
        spec: &SpawnSpec,
        size: TermSize,
        ctx: PaneCtx,
        preload: Option<&str>,
    ) -> anyhow::Result<Self> {
        let PaneCtx { options, sink: out, events: internal, recorder } = ctx;
        let pty = native_pty_system().openpty(pty_size(size))?;

        let program = spec.program.clone().or_else(|| spec.integration.then(crate::integration::default_shell).flatten());
        let launch = match (&program, spec.integration) {
            (Some(p), true) => crate::integration::launch_for(p, &spec.args),
            _ => None,
        };
        let mut cmd = match &program {
            Some(p) => {
                let mut c = CommandBuilder::new(p);
                c.args(&spec.args);
                if let Some(l) = &launch {
                    c.args(&l.args);
                }
                c
            }
            None => CommandBuilder::new_default_prog(),
        };
        if let Some(l) = &launch {
            for (k, v) in &l.env {
                cmd.env(k, v);
            }
        }
        if let Some(cwd) = &spec.cwd {
            cmd.cwd(cwd);
        }
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        cmd.env("TERM_PROGRAM", "zhell");
        cmd.env("TERM_PROGRAM_VERSION", env!("CARGO_PKG_VERSION"));
        for (k, v) in &spec.env {
            cmd.env(k, v);
        }

        let mut child = pty.slave.spawn_command(cmd)?;
        drop(pty.slave);
        let shell_pid = child.process_id();
        let killer = child.clone_killer();
        let reader = pty.master.try_clone_reader()?;
        let writer: PtyWriter = Arc::new(Mutex::new(pty.master.take_writer()?));
        let size_cell = Arc::new(Mutex::new(size));

        let title = Arc::new(Mutex::new(None));
        let listener = Listener {
            pane: id,
            out: out.clone(),
            writer: writer.clone(),
            size: size_cell.clone(),
            title: title.clone(),
        };
        let mut term = Term::new(term_config(options), &grid_size(size), listener);
        if let Some(text) = preload {
            let mut parser: Processor = Processor::new();
            parser.advance(&mut term, text.as_bytes());
        }
        let term = Arc::new(FairMutex::new(term));

        let cwd = Arc::new(Mutex::new(None));
        let blocks = Arc::new(Mutex::new(crate::blocks::Blocks::default()));
        let images = Arc::new(Mutex::new(crate::images::Images::default()));
        let cast = Arc::new(Mutex::new(None));
        let state = ReaderState {
            id,
            writer: writer.clone(),
            size: size_cell.clone(),
            images: images.clone(),
            term: term.clone(),
            cwd: cwd.clone(),
            blocks: blocks.clone(),
            out: out.clone(),
            events: internal,
            recorder,
            cast: cast.clone(),
        };
        thread::Builder::new().name(format!("pane-{}-reader", id.0)).spawn(move || {
            read_loop(reader, &state);
            let code = child.wait().ok().map(|s| s.exit_code() as i32);
            let _ = state.events.send(Event::Exited(id, code));
        })?;

        Ok(Self {
            id,
            sink: out,
            title,
            blocks,
            find: None,
            folds: Default::default(),
            view: None,
            force_full: false,
            images,
            cast,
            term,
            master: pty.master,
            writer,
            killer,
            size: size_cell,
            cwd,
            shell_pid,
            ports: Default::default(),
            start_dir: spec.cwd.clone().or_else(|| std::env::current_dir().ok().map(|d| d.display().to_string())),
            inflight: None,
            dirty: true,
            next_seq: 1,
        })
    }

    pub fn start_recording(&self, path: &std::path::Path) -> std::io::Result<()> {
        let size = *self.size.lock();
        let mut cast = crate::cast::Cast::create(path, size.cols, size.rows, self.title.lock().as_deref())?;
        {
            let term = self.term.lock();
            let grid = term.grid();
            let cursor = grid.cursor.point;
            let line: String = (0..cursor.column.0.min(grid.columns())).map(|c| grid[cursor.line][Column(c)].c).collect();
            cast.output(line.as_bytes());
        }
        *self.cast.lock() = Some(cast);
        Ok(())
    }

    pub fn stop_recording(&self) -> Option<std::path::PathBuf> {
        self.cast.lock().take().map(|c| c.path)
    }

    pub fn write(&self, bytes: &[u8]) {
        let mut w = self.writer.lock();
        if let Err(e) = w.write_all(bytes).and_then(|()| w.flush()) {
            log::warn!("pane {}: write failed: {e}", self.id.0);
        }
    }

    pub fn resize(&mut self, size: TermSize) {
        let old = *self.size.lock();
        if (size.cols, size.rows) != (old.cols, old.rows)
            && let Some(c) = self.cast.lock().as_mut()
        {
            c.resize(size.cols, size.rows);
        }
        *self.size.lock() = size;
        if let Err(e) = self.master.resize(pty_size(size)) {
            log::warn!("pane {}: pty resize failed: {e}", self.id.0);
        }
        self.term.lock().resize(grid_size(size));
        self.dirty = true;
    }

    pub fn scroll(&mut self, scroll: Scroll) {
        let mut term = self.term.lock();
        match (scroll, &self.view) {
            (Scroll::Delta(delta), Some(v)) => {
                let rows = term.screen_lines() as i32;
                let offset = term.grid().display_offset() as i32;
                let top = -(term.grid().history_size() as i32);
                let anchor = crate::folds::scroll_anchor(rows - 1 - offset, delta, top + rows - 1, rows - 1, &v.hidden);
                let new_offset = rows - 1 - anchor;
                term.scroll_display(Scroll::Delta(new_offset - offset));
            }
            _ => term.scroll_display(scroll),
        }
        drop(term);
        self.dirty = true;
    }

    pub fn set_fold(&mut self, block: u32, folded: bool) {
        self.folds.set(block, folded);
        self.force_full = true;
        self.dirty = true;
    }

    pub fn set_focus(&mut self, focused: bool) {
        let report = {
            let mut t = self.term.lock();
            t.is_focused = focused;
            t.mode().contains(alacritty_terminal::term::TermMode::FOCUS_IN_OUT)
        };
        if report {
            self.write(if focused { b"\x1b[I" } else { b"\x1b[O" });
        }
        self.dirty = true;
    }

    fn grid_point(&self, term: &Term<Listener>, row: i32, col: u16) -> Point {
        let offset = term.grid().display_offset() as i32;
        let top = -(term.grid().history_size() as i32);
        let bottom = term.screen_lines() as i32 - 1;
        let line = match &self.view {
            Some(v) => v.line_of(row),
            None => row - offset,
        }
        .clamp(top, bottom);
        let col = (col as usize).min(term.columns().saturating_sub(1));
        Point::new(Line(line), Column(col))
    }

    pub fn select_start(&mut self, row: u16, col: u16, right_half: bool, kind: SelectKind) {
        let mut term = self.term.lock();
        let point = self.grid_point(&term, row as i32, col);
        let ty = match kind {
            SelectKind::Simple => SelectionType::Simple,
            SelectKind::Block => SelectionType::Block,
            SelectKind::Word => SelectionType::Semantic,
            SelectKind::Line => SelectionType::Lines,
        };
        term.selection = Some(Selection::new(ty, point, side(right_half)));
        self.dirty = true;
    }

    pub fn select_update(&mut self, row: i32, col: u16, right_half: bool) {
        let mut term = self.term.lock();
        let point = self.grid_point(&term, row, col);
        if let Some(sel) = term.selection.as_mut() {
            sel.update(point, side(right_half));
        }
        self.dirty = true;
    }

    pub fn copy_mode(&mut self, cmd: zhell_proto::CopyCmd) -> Option<String> {
        use alacritty_terminal::term::TermMode;
        use alacritty_terminal::vi_mode::ViMotion as M;
        use zhell_proto::CopyCmd as C;
        let mut term = self.term.lock();
        self.dirty = true;
        let active = term.mode().contains(TermMode::VI);
        match cmd {
            C::Enter => {
                if !active {
                    term.toggle_vi_mode();
                }
                return None;
            }
            _ if !active => return None,
            C::Exit | C::Yank => {
                let text = (cmd == C::Yank).then(|| term.selection_to_string()).flatten().filter(|s| !s.is_empty());
                term.toggle_vi_mode();
                term.selection = None;
                term.scroll_display(Scroll::Bottom);
                return text;
            }
            C::Select(kind) => {
                let ty = match kind {
                    SelectKind::Simple => SelectionType::Simple,
                    SelectKind::Block => SelectionType::Block,
                    SelectKind::Word => SelectionType::Semantic,
                    SelectKind::Line => SelectionType::Lines,
                };
                if term.selection.as_ref().is_some_and(|s| s.ty == ty) {
                    term.selection = None;
                } else {
                    let point = term.vi_mode_cursor.point;
                    let mut sel = Selection::new(ty, point, Side::Left);
                    sel.include_all();
                    term.selection = Some(sel);
                }
                return None;
            }
            C::Top | C::Bottom => {
                let line = if cmd == C::Top { term.topmost_line() } else { term.bottommost_line() };
                term.vi_goto_point(Point::new(line, Column(0)));
                return None;
            }
            C::HalfPageUp | C::HalfPageDown | C::PageUp | C::PageDown => {
                let rows = term.screen_lines() as i32;
                let lines = match cmd {
                    C::HalfPageUp => rows / 2,
                    C::HalfPageDown => -rows / 2,
                    C::PageUp => rows,
                    _ => -rows,
                };
                term.scroll_display(Scroll::Delta(lines));
                let cursor = term.vi_mode_cursor.scroll(&term, lines);
                term.vi_goto_point(cursor.point);
                return None;
            }
            C::PrevPrompt | C::NextPrompt => {
                let top_line = term.vi_mode_cursor.point.line.0;
                if let Some(target) = crate::blocks::jump_target(term.grid(), top_line, cmd == C::NextPrompt) {
                    term.vi_goto_point(Point::new(Line(target), Column(0)));
                }
                return None;
            }
            _ => {}
        }
        let motion = match cmd {
            C::Up => M::Up,
            C::Down => M::Down,
            C::Left => M::Left,
            C::Right => M::Right,
            C::LineStart => M::First,
            C::LineEnd => M::Last,
            C::FirstNonBlank => M::FirstOccupied,
            C::WordNext => M::SemanticRight,
            C::WordPrev => M::SemanticLeft,
            C::WordEnd => M::SemanticRightEnd,
            C::BigWordNext => M::WordRight,
            C::BigWordPrev => M::WordLeft,
            C::BigWordEnd => M::WordRightEnd,
            C::ScreenTop => M::High,
            C::ScreenMiddle => M::Middle,
            C::ScreenBottom => M::Low,
            C::ParagraphUp => M::ParagraphUp,
            C::ParagraphDown => M::ParagraphDown,
            _ => M::Bracket,
        };
        term.vi_motion(motion);
        let point = term.vi_mode_cursor.point;
        term.scroll_to_point(point);
        None
    }

    pub fn select_clear(&mut self) {
        let mut term = self.term.lock();
        if term.selection.take().is_some() {
            self.dirty = true;
        }
    }

    pub fn selection_text(&self) -> Option<String> {
        self.term.lock().selection_to_string().filter(|s| !s.is_empty())
    }

    pub fn set_options(&mut self, options: HostOptions) {
        self.term.lock().set_options(term_config(options));
        self.dirty = true;
    }

    pub fn working_dir(&self) -> Option<std::path::PathBuf> {
        #[cfg(target_os = "linux")]
        if let Some(pgid) = self.master.process_group_leader()
            && let Ok(dir) = std::fs::read_link(format!("/proc/{pgid}/cwd"))
        {
            return Some(dir);
        }
        self.cwd.lock().clone().or_else(|| self.start_dir.clone()).map(Into::into)
    }

    pub fn foreground_name(&self) -> Option<String> {
        #[cfg(target_os = "linux")]
        {
            let pgid = self.master.process_group_leader()?;
            let comm = std::fs::read_to_string(format!("/proc/{pgid}/comm")).ok()?;
            Some(comm.trim().to_owned())
        }
        #[cfg(not(target_os = "linux"))]
        None
    }

    pub fn resolve_path(&self, path: &str) -> Option<String> {
        use std::path::{Path, PathBuf};
        let expanded: PathBuf = match path.strip_prefix("~/").or_else(|| path.strip_prefix("~\\")) {
            Some(rest) => std::env::home_dir()?.join(rest),
            None if path == "~" => std::env::home_dir()?,
            None => PathBuf::from(path),
        };
        let full = if expanded.is_absolute() { expanded } else { self.working_dir()?.join(expanded) };
        let full = full.canonicalize().ok()?;
        Path::exists(&full).then(|| full.display().to_string())
    }

    pub fn kill(&mut self) {
        let _ = self.killer.kill();
    }

    pub fn take_frame(&mut self, full: bool) -> FrameDiff {
        let seq = self.next_seq;
        self.next_seq += 1;
        self.dirty = false;
        let full = full || std::mem::take(&mut self.force_full);
        build_frame(
            &mut self.term.lock(),
            self.id,
            seq,
            full,
            Some(&self.blocks.lock()),
            self.find.as_mut(),
            Some(&mut self.folds),
            Some(&mut self.view),
        )
    }

    pub fn find(&mut self, query: &str, regex: bool) {
        use alacritty_terminal::term::search::RegexSearch;
        if query.is_empty() {
            self.find = Some(Find { regex: None, current: None });
            self.dirty = true;
            return;
        }
        let pattern = if regex { query.to_owned() } else { regex::escape(query) };
        self.find = Some(Find { regex: RegexSearch::new(&pattern).ok(), current: None });

        self.dirty = true;
        let origin = self.bottom_right();
        self.find_from(origin, alacritty_terminal::index::Direction::Left, Side::Right);
    }

    fn bottom_right(&self) -> Point {
        let term = self.term.lock();
        Point::new(Line(term.screen_lines() as i32 - 1), Column(term.columns().saturating_sub(1)))
    }

    pub fn find_next(&mut self, older: bool) {
        use alacritty_terminal::index::Direction;
        match self.find.as_ref().and_then(|f| f.current.clone()) {
            None => {
                let origin = self.bottom_right();
                self.find_from(origin, Direction::Left, Side::Right);
            }
            Some(cur) if older => self.find_from(*cur.start(), Direction::Left, Side::Left),
            Some(cur) => self.find_from(*cur.end(), Direction::Right, Side::Right),
        }
    }

    fn find_from(&mut self, origin: Point, dir: alacritty_terminal::index::Direction, side: Side) {
        use alacritty_terminal::index::{Boundary, Direction};
        let Some(f) = self.find.as_mut() else { return };
        let Some(regex) = f.regex.as_mut() else { return };
        let mut term = self.term.lock();

        let origin = match (f.current.is_some(), dir) {
            (true, Direction::Left) => origin.sub(&*term, Boundary::None, 1),
            (true, Direction::Right) => origin.add(&*term, Boundary::None, 1),
            _ => origin,
        };
        let found = term.search_next(regex, origin, dir, side, None);
        log::debug!("find from {origin:?} {dir:?}: {found:?}");
        if let Some(m) = found {
            let rows = term.screen_lines() as i32;
            let offset = term.grid().display_offset() as i32;
            let line = m.start().line.0;
            if line < -offset || line > -offset + rows - 1 {
                let wanted = (rows / 3 - line).clamp(0, term.grid().history_size() as i32);
                term.scroll_display(Scroll::Delta(wanted - offset));
            }
            f.current = Some(m);
        }
        drop(term);
        self.dirty = true;
    }

    pub fn find_close(&mut self) {
        self.find = None;
        self.dirty = true;
    }

    pub fn block_cells(&self, block: u32) -> Option<Vec<Vec<Cell>>> {
        const MAX: i32 = 20_000;
        let term = self.term.lock();
        let grid = term.grid();
        let (first, last) = crate::blocks::block_range(grid, block)?;
        let first = first.max(last - MAX + 1);
        let colors = term.colors();
        Some((first..=last).map(|l| (0..grid.columns()).map(|c| convert_cell(&grid[Line(l)][Column(c)], colors)).collect()).collect())
    }

    pub fn block_output(&self, block: u32) -> Option<String> {
        crate::blocks::output_text(self.term.lock().grid(), block)
    }

    pub fn jump_block(&mut self, forward: bool) {
        let mut term = self.term.lock();
        let top_line = -(term.grid().display_offset() as i32);
        if let Some(target) = crate::blocks::jump_target(term.grid(), top_line, forward) {
            term.scroll_display(Scroll::Delta(top_line - target));
        } else if forward {
            term.scroll_display(Scroll::Bottom);
        }
        drop(term);
        self.dirty = true;
    }

    pub fn cwd(&self) -> Option<String> {
        self.cwd.lock().clone()
    }

    pub fn snapshot(&self) -> crate::snapshot::PaneSnap {
        use alacritty_terminal::term::cell::Flags;
        let term = self.term.lock();
        let grid = term.grid();
        let top = -(grid.history_size() as i32);
        let bottom = term.screen_lines() as i32;
        let first = (bottom - crate::snapshot::MAX_LINES as i32).max(top);
        let mut lines: Vec<String> = (first..bottom)
            .map(|l| {
                let row = &grid[Line(l)];
                let text: String = (0..term.columns())
                    .map(|c| &row[Column(c)])
                    .filter(|c| !c.flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER))
                    .map(|c| if c.c == '\t' { ' ' } else { c.c })
                    .collect();
                text.trim_end().to_owned()
            })
            .collect();
        while lines.last().is_some_and(String::is_empty) {
            lines.pop();
        }
        let cwd = self
            .working_dir()
            .map(|d| d.display().to_string())
            .or_else(|| self.cwd());
        crate::snapshot::PaneSnap {
            id: self.id,
            cwd,
            title: self.title.lock().clone(),
            size: *self.size.lock(),
            lines,
        }
    }
}

pub struct View {
    pub rows: Vec<crate::folds::RowSrc>,

    pub hidden: Vec<crate::folds::Hidden>,
}

impl View {
    pub fn identity(rows: usize, display_offset: usize) -> Self {
        let rows = (0..rows as i32).map(|r| crate::folds::RowSrc::Line(r - display_offset as i32)).collect();
        Self { rows, hidden: Vec::new() }
    }

    pub fn row_of(&self, line: i32) -> i32 {
        use crate::folds::RowSrc;
        if let Some(r) = crate::folds::row_of(&self.rows, &self.hidden, line) {
            return r as i32;
        }
        let first = self.rows.iter().find_map(|r| match r {
            RowSrc::Line(l) => Some(*l),
            _ => None,
        });
        let last = self.rows.iter().rev().find_map(|r| match r {
            RowSrc::Line(l) => Some(*l),
            _ => None,
        });
        match (first, last) {
            (Some(f), _) if line < f => -(f - line),
            (_, Some(l)) if line > l => self.rows.len() as i32 - 1 + (line - l),
            _ => -1,
        }
    }

    pub fn line_of(&self, row: i32) -> i32 {
        use crate::folds::RowSrc;
        let n = self.rows.len() as i32;
        let clamp = row.clamp(0, (n - 1).max(0));
        let base = match self.rows.get(clamp as usize) {
            Some(RowSrc::Line(l)) => *l,
            Some(RowSrc::Fold { block, .. }) => self.hidden.iter().find(|h| h.block == *block).map_or(0, |h| h.first),
            None => 0,
        };
        base + (row - clamp)
    }

    pub fn lines(&self) -> Vec<(i32, i32)> {
        self.rows
            .iter()
            .enumerate()
            .filter_map(|(r, src)| match src {
                crate::folds::RowSrc::Line(l) => Some((r as i32, *l)),
                _ => None,
            })
            .collect()
    }
}

#[allow(clippy::too_many_arguments)]
pub fn build_frame<T: EventListener>(
    term: &mut Term<T>,
    pane: PaneId,
    seq: u64,
    force_full: bool,
    blocks: Option<&crate::blocks::Blocks>,
    find: Option<&mut Find>,
    folds: Option<&mut crate::folds::Folds>,
    view_out: Option<&mut Option<View>>,
) -> FrameDiff {
        use crate::folds::RowSrc;
        let rows = term.screen_lines();
        let cols = term.columns();
        let display_offset = term.grid().display_offset();

        let alt = term.mode().contains(alacritty_terminal::term::TermMode::ALT_SCREEN);
        let view = match folds {
            Some(f) if !f.is_empty() && !alt => {
                let hidden = f.hidden(term.grid());
                if hidden.is_empty() {
                    View::identity(rows, display_offset)
                } else {
                    let bottom_line = rows as i32 - 1 - display_offset as i32;
                    let top = -(term.grid().history_size() as i32);
                    View { rows: crate::folds::row_map(rows, bottom_line, top, &hidden), hidden }
                }
            }
            _ => View::identity(rows, display_offset),
        };
        let folded = !view.hidden.is_empty();
        let (full, damaged): (bool, Vec<usize>) = match term.damage() {
            _ if force_full || folded => (true, (0..rows).collect()),
            TermDamage::Full => (true, (0..rows).collect()),
            TermDamage::Partial(it) => (false, it.map(|d| d.line).collect()),
        };
        term.reset_damage();

        let grid = term.grid();
        let colors = term.colors();
        let (top, bottom) = (-(grid.history_size() as i32), rows as i32 - 1);
        let mut fold_rows = Vec::new();
        let lines = damaged
            .into_iter()
            .filter(|&r| r < rows)
            .map(|row| {
                let cells = match view.rows[row] {
                    RowSrc::Line(l) if l >= top && l <= bottom => {
                        (0..cols).map(|c| convert_cell(&grid[Line(l)][Column(c)], colors)).collect()
                    }
                    RowSrc::Line(_) => vec![Cell::default(); cols],
                    RowSrc::Fold { block, hidden } => {
                        fold_rows.push(zhell_proto::FoldRow { row: row as u16, block, hidden });
                        fold_cells(cols, hidden)
                    }
                };
                LineUpdate { row: row as u16, cells }
            })
            .collect();

        let selection = term.selection.as_ref().and_then(|s| s.to_range(term)).map(|r| SelectionSpan {
            start_row: view.row_of(r.start.line.0),
            start_col: r.start.column.0 as u16,
            end_row: view.row_of(r.end.line.0),
            end_col: r.end.column.0 as u16,
            block: r.is_block,
        });

        let line_rows = view.lines();
        let blocks = blocks.map(|b| b.visible_view(grid, &line_rows, display_offset == 0)).unwrap_or_default();
        let images = image_rows(grid, &line_rows);
        let find = find.map(|f| find_state(term, f, &view));

        let content_cursor = term.renderable_content().cursor;
        let style = term.cursor_style();
        let cursor_row = crate::folds::row_of(&view.rows, &view.hidden, content_cursor.point.line.0)
            .filter(|_| view.hidden.iter().all(|h| content_cursor.point.line.0 < h.first || content_cursor.point.line.0 > h.last));
        let shape = match cursor_row {
            Some(_) => convert_shape(content_cursor.shape),
            None => CursorShape::Hidden,
        };
        let frame = FrameDiff {
            pane,
            seq,
            cols: cols as u16,
            rows: rows as u16,
            history_len: grid.history_size() as u32,
            display_offset: display_offset as u32,
            cursor: CursorState {
                row: cursor_row.unwrap_or(0) as u16,
                col: content_cursor.point.column.0 as u16,
                shape,
                blinking: style.blinking,
            },
            modes: term.mode().bits(),
            selection,
            blocks,
            images,
            find,
            folds: fold_rows,
            full,
            lines,
        };
        if let Some(out) = view_out {
            *out = folded.then_some(view);
        }
        frame
}

fn fold_cells(cols: usize, hidden: u32) -> Vec<Cell> {
    let text = format!("  ▸ {hidden} more line{} — click to show", if hidden == 1 { "" } else { "s" });
    let mut cells = vec![Cell::default(); cols];
    for (cell, ch) in cells.iter_mut().zip(text.chars()) {
        cell.ch = ch;
        cell.flags = zhell_proto::flags::DIM | zhell_proto::flags::ITALIC;
    }
    cells
}

fn find_state<T>(term: &Term<T>, f: &mut Find, view: &View) -> zhell_proto::FindState {
    use alacritty_terminal::term::search::{Match, RegexIter};
    let span = |m: &Match| SelectionSpan {
        start_row: view.row_of(m.start().line.0),
        start_col: m.start().column.0 as u16,
        end_row: view.row_of(m.end().line.0),
        end_col: m.end().column.0 as u16,
        block: false,
    };
    let Some(regex) = f.regex.as_mut() else {
        return zhell_proto::FindState { matches: Vec::new(), current: None, found: false, invalid: true };
    };
    let lines = view.lines();
    let rows = view.rows.len() as i32;
    let (Some(&(_, first)), Some(&(_, last))) = (lines.first(), lines.last()) else {
        return zhell_proto::FindState { matches: Vec::new(), current: None, found: f.current.is_some(), invalid: false };
    };
    let bottom_line = last.min(term.screen_lines() as i32 - 1);
    let top = Point::new(Line(first.max(-(term.grid().history_size() as i32))), Column(0));
    let bottom = Point::new(Line(bottom_line), Column(term.columns().saturating_sub(1)));
    let hidden = |l: i32| view.hidden.iter().any(|h| l >= h.first && l <= h.last);
    let matches: Vec<SelectionSpan> = RegexIter::new(top, bottom, alacritty_terminal::index::Direction::Right, term, regex)
        .take(500)
        .filter(|m| !hidden(m.start().line.0))
        .map(|m| span(&m))
        .collect();
    let current = f.current.as_ref().filter(|m| !hidden(m.start().line.0)).map(span).filter(|s| s.end_row >= 0 && s.start_row < rows);
    zhell_proto::FindState { matches, current, found: f.current.is_some(), invalid: false }
}

fn image_rows(grid: &alacritty_terminal::Grid<TermCell>, view: &[(i32, i32)]) -> Vec<zhell_proto::ImageRow> {
    let (top, bottom) = (-(grid.history_size() as i32), grid.screen_lines() as i32 - 1);
    let mut out = Vec::new();
    for &(row, line) in view {
        if line < top || line > bottom {
            continue;
        }
        let cells = &grid[Line(line)];
        for c in 0..grid.columns() {
            let cell = &cells[Column(c)];
            if cell.extra.is_none() {
                continue;
            }
            let Some(link) = cell.hyperlink() else { continue };
            let Some(rest) = link.uri().strip_prefix(crate::images::TAG_PREFIX) else { continue };
            if let Some((id, image_row)) = rest.split_once('/')
                && let (Ok(id), Ok(image_row)) = (id.parse(), image_row.parse())
            {
                out.push(zhell_proto::ImageRow { row: row as u16, col: c as u16, id, image_row });
                break;
            }
        }
    }
    out
}

fn read_loop(mut reader: Box<dyn Read + Send>, st: &ReaderState) {
    let (id, term, cwd_slot, blocks, out, internal) = (st.id, &*st.term, &*st.cwd, &*st.blocks, &st.out, &st.events);
    let recorder = st.recorder.as_ref();
    let mut parser: Processor = Processor::new();
    let mut prescan = OscPrescanner::new();
    let mut events = Vec::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        };
        let bytes = &buf[..n];
        if let Some(c) = st.cast.lock().as_mut() {
            c.output(bytes);
        }
        events.clear();
        prescan.scan(bytes, &mut events);

        let mut marks = Vec::new();
        let mut image_msgs = Vec::new();
        {
            let mut t = term.lock();
            let mut pos = 0;
            for (end, ev) in events.drain(..) {
                parser.advance(&mut *t, &bytes[pos..end]);
                pos = end;

                if let OscEvent::KittyGraphics(payload) | OscEvent::Sixel(payload) = &ev {
                    let size = *st.size.lock();
                    let g = crate::images::Geometry { cell_w: size.cell_width, cell_h: size.cell_height, cols: t.columns() as u16 };
                    let mut imgs = st.images.lock();
                    let effect = if matches!(ev, OscEvent::KittyGraphics(_)) { imgs.kitty(payload, g) } else { imgs.sixel(payload, g) };
                    if !effect.inject.is_empty() {
                        parser.advance(&mut *t, effect.inject.as_bytes());
                    }
                    if let Some(reply) = effect.reply {
                        let mut w = st.writer.lock();
                        let _ = w.write_all(reply.as_bytes()).and_then(|()| w.flush());
                    }
                    if let Some(img_id) = effect.placed
                        && let Some(img) = imgs.images.get(&img_id)
                    {
                        image_msgs.push(image_msg(id, img_id, img));
                    }
                    continue;
                }

                let mut b = blocks.lock();
                if let Some(inject) = b.on_event(&ev) {
                    parser.advance(&mut *t, inject.as_bytes());
                }
                if let (Some(rec), Some(done)) = (recorder, b.take_finished()) {
                    record(rec, &done, t.grid());
                }
                let abs_line = (t.grid().history_size() as i64 + t.grid().cursor.point.line.0 as i64)
                    .max(0) as u64;
                marks.push((abs_line, ev));
            }
            parser.advance(&mut *t, &bytes[pos..]);
        }

        for msg in image_msgs {
            out.send(msg);
        }
        for (abs_line, ev) in marks {
            let kind = match ev {
                OscEvent::PromptStart => MarkKind::PromptStart,
                OscEvent::CommandStart => MarkKind::CommandStart,
                OscEvent::OutputStart => MarkKind::OutputStart,
                OscEvent::CommandFinished { exit_code } => MarkKind::CommandFinished { exit_code },
                OscEvent::CommandText(s) => MarkKind::CommandText(s),
                OscEvent::KittyGraphics(_) | OscEvent::Sixel(_) => continue,
                OscEvent::Cwd(cwd) => {
                    *cwd_slot.lock() = Some(cwd.clone());
                    out.send(ServerMsg::Cwd { pane: id, cwd });
                    continue;
                }
                OscEvent::Ready => {
                    out.send(ServerMsg::Ready { pane: id });
                    continue;
                }

                OscEvent::RemoteCwd { host, path } => {
                    out.send(ServerMsg::RemoteCwd { pane: id, host, cwd: path });
                    continue;
                }
            };
            out.send(ServerMsg::Mark { pane: id, mark: ShellMark { kind, abs_line } });
        }
        if internal.send(Event::Dirty(id)).is_err() {
            break;
        }
    }
}

fn record(rec: &Recorder, b: &crate::blocks::BlockMeta, grid: &alacritty_terminal::Grid<TermCell>) {
    let Some(cmd) = b.cmd.as_deref().map(str::trim).filter(|c| !c.is_empty()) else { return };
    if rec.settings.excluded(b.cwd.as_deref()) {
        return;
    }
    let ms = |t: Option<std::time::SystemTime>| {
        t.and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_millis() as u64)
    };
    let block = zhell_history::NewBlock {
        host: b.host.clone().unwrap_or_else(|| (*rec.host).clone()),
        cwd: b.cwd.clone(),
        cmd: cmd.to_owned(),
        exit_code: match b.state {
            zhell_proto::BlockState::Done { exit } => exit,
            _ => None,
        },
        started_ms: ms(b.started),
        ended_ms: ms(b.finished),
        output: crate::blocks::output_text(grid, b.id).unwrap_or_default(),
    };
    let _ = rec.tx.send(crate::history::HistoryCmd::Insert(block));
}

pub(crate) fn image_msg(pane: PaneId, id: u32, img: &crate::images::Image) -> ServerMsg {
    ServerMsg::Image { pane, id, width: img.width, height: img.height, cols: img.cols, rows: img.rows, rgba: img.rgba.clone() }
}

fn term_config(o: HostOptions) -> Config {
    let shape = match o.cursor_shape {
        CursorShape::Beam => ansi::CursorShape::Beam,
        CursorShape::Underline => ansi::CursorShape::Underline,
        _ => ansi::CursorShape::Block,
    };
    Config {
        scrolling_history: o.scrollback_lines as usize,
        default_cursor_style: ansi::CursorStyle { shape, blinking: o.cursor_blink },
        kitty_keyboard: true,
        ..Config::default()
    }
}

fn side(right_half: bool) -> Side {
    if right_half { Side::Right } else { Side::Left }
}

fn pty_size(s: TermSize) -> PtySize {
    PtySize {
        rows: s.rows.max(1),
        cols: s.cols.max(1),
        pixel_width: s.cols.saturating_mul(s.cell_width),
        pixel_height: s.rows.saturating_mul(s.cell_height),
    }
}

pub(crate) fn grid_size(s: TermSize) -> GridSize {
    GridSize { cols: s.cols.max(1) as usize, rows: s.rows.max(1) as usize }
}

fn convert_color(c: ansi::Color, colors: &alacritty_terminal::term::color::Colors) -> Color {
    match c {
        ansi::Color::Spec(rgb) => Color::Rgb(rgb.r, rgb.g, rgb.b),
        ansi::Color::Indexed(i) => match colors[i as usize] {
            Some(rgb) => Color::Rgb(rgb.r, rgb.g, rgb.b),
            None => Color::Indexed(i),
        },
        ansi::Color::Named(n) => match colors[n as usize] {
            Some(rgb) => Color::Rgb(rgb.r, rgb.g, rgb.b),
            None => Color::Named(n as u16),
        },
    }
}

fn convert_cell(c: &TermCell, colors: &alacritty_terminal::term::color::Colors) -> Cell {
    Cell {
        ch: c.c,
        zerowidth: c.zerowidth().map(<[char]>::to_vec).unwrap_or_default(),
        fg: convert_color(c.fg, colors),
        bg: convert_color(c.bg, colors),
        underline_color: c.underline_color().map(|u| convert_color(u, colors)),
        flags: c.flags.bits(),

        hyperlink: c.hyperlink().map(|h| h.uri().to_owned()).filter(|u| !u.starts_with("zhell:")),
    }
}

fn convert_shape(s: ansi::CursorShape) -> CursorShape {
    match s {
        ansi::CursorShape::Block => CursorShape::Block,
        ansi::CursorShape::Underline => CursorShape::Underline,
        ansi::CursorShape::Beam => CursorShape::Beam,
        ansi::CursorShape::HollowBlock => CursorShape::HollowBlock,
        ansi::CursorShape::Hidden => CursorShape::Hidden,
    }
}
