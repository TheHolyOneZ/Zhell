use std::collections::VecDeque;
use std::time::{SystemTime, UNIX_EPOCH};

use alacritty_terminal::grid::{Dimensions, Grid};
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::{Cell, Flags};
use zhell_core::prescan::OscEvent;
use zhell_proto::{BlockSpan, BlockState};

const MAX_BLOCKS: usize = 10_000;

const FULL_SCAN_ABOVE: i32 = 3000;
const FAST_SCAN_CELLS: usize = 4;
pub const TAG_PREFIX: &str = "zhell:b/";

type Found = Option<(u32, i32, i32)>;
type DeepScan = ((usize, usize), Found);

const FAR_ABOVE: i32 = -1_000_000;

#[derive(Clone, Debug)]
pub struct BlockMeta {
    pub id: u32,
    pub cmd: Option<String>,
    pub cwd: Option<String>,

    pub host: Option<String>,
    pub state: BlockState,
    pub started: Option<SystemTime>,
    pub finished: Option<SystemTime>,
}

#[derive(Default)]
pub struct Blocks {
    list: VecDeque<BlockMeta>,
    next_id: u32,

    tag_open: bool,
    cwd: Option<String>,

    host: Option<String>,
    just_finished: Option<u32>,

    deep_cache: std::sync::Mutex<Option<DeepScan>>,
}

fn open_tag(id: u32) -> String {
    format!("\x1b]8;id=zb{id};{TAG_PREFIX}{id}\x1b\\")
}

const CLOSE_TAG: &str = "\x1b]8;;\x1b\\";

fn ms(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

impl Blocks {
    fn current(&mut self) -> Option<&mut BlockMeta> {
        self.list.back_mut()
    }

    pub fn get(&self, id: u32) -> Option<&BlockMeta> {
        self.list.iter().rev().find(|b| b.id == id)
    }

    pub fn take_finished(&mut self) -> Option<BlockMeta> {
        let id = self.just_finished.take()?;
        self.get(id).cloned()
    }

    fn set_cwd(&mut self, host: Option<String>, cwd: &str) {
        self.cwd = Some(cwd.to_owned());
        self.host = host.clone();
        if let Some(b) = self.current()
            && b.state == BlockState::Editing
        {
            b.cwd = Some(cwd.to_owned());
            b.host = host;
        }
    }

    pub fn on_event(&mut self, ev: &OscEvent) -> Option<String> {
        match ev {
            OscEvent::PromptStart => {
                let mut inject = String::new();
                if self.tag_open {
                    inject.push_str(CLOSE_TAG);
                }

                let reuse = matches!(self.list.back(), Some(b) if b.state == BlockState::Editing);
                if !reuse {
                    if let Some(b) = self.current()
                        && b.state == BlockState::Running
                    {
                        b.state = BlockState::Done { exit: None };
                        b.finished = Some(SystemTime::now());
                    }
                    self.next_id += 1;
                    self.list.push_back(BlockMeta {
                        id: self.next_id,
                        cmd: None,
                        cwd: self.cwd.clone(),
                        host: self.host.clone(),
                        state: BlockState::Editing,
                        started: None,
                        finished: None,
                    });
                    if self.list.len() > MAX_BLOCKS {
                        self.list.pop_front();
                    }
                }
                let id = self.list.back().map_or(0, |b| b.id);
                inject.push_str(&open_tag(id));
                self.tag_open = true;
                Some(inject)
            }
            OscEvent::CommandStart | OscEvent::Ready | OscEvent::KittyGraphics(_) | OscEvent::Sixel(_) => None,
            OscEvent::CommandText(cmd) => {
                if let Some(b) = self.current()
                    && b.state == BlockState::Editing
                {
                    b.cmd = Some(cmd.clone());
                }
                None
            }
            OscEvent::OutputStart => {
                if let Some(b) = self.current()
                    && b.state == BlockState::Editing
                {
                    b.state = BlockState::Running;
                    b.started = Some(SystemTime::now());
                }
                self.tag_open.then(|| {
                    self.tag_open = false;
                    CLOSE_TAG.to_owned()
                })
            }
            OscEvent::CommandFinished { exit_code } => {
                if let Some(b) = self.current()
                    && b.state == BlockState::Running
                {
                    b.state = BlockState::Done { exit: *exit_code };
                    b.finished = Some(SystemTime::now());
                    self.just_finished = Some(b.id);
                }
                None
            }
            OscEvent::Cwd(cwd) => {
                self.set_cwd(None, cwd);
                None
            }
            OscEvent::RemoteCwd { host, path } => {
                self.set_cwd(Some(host.clone()), path);
                None
            }
        }
    }

    fn deep_scan(&self, grid: &Grid<Cell>, from: i32) -> Found {
        let key = (grid.history_size(), from as isize as usize);
        let mut cache = self.deep_cache.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((k, v)) = *cache
            && k == key
        {
            return v;
        }
        let top = -(grid.history_size() as i32);
        let mut line = from;
        let mut hit: Found = None;
        while line >= top {
            let tag = match hit {
                None => row_tag_prefix(grid, Line(line), FAST_SCAN_CELLS),
                Some(_) => row_tag(grid, Line(line)),
            };
            match (tag, hit) {
                (Some(id), None) => hit = Some((id, line, line)),
                (Some(id), Some((h, _, last))) if id == h => hit = Some((h, line, last)),
                (_, Some(_)) => break,
                (None, None) => {}
            }
            line -= 1;
        }
        *cache = Some((key, hit));
        hit
    }

    fn span(&self, id: u32, first: i32, last: i32) -> Option<BlockSpan> {
        let m = self.get(id)?;
        let duration_ms = match (m.started, m.finished) {
            (Some(s), Some(f)) => f.duration_since(s).ok().map(|d| d.as_millis() as u64),
            _ => None,
        };
        Some(BlockSpan {
            id,
            prompt_row: first,
            output_row: last + 1,
            cmd: m.cmd.clone(),
            state: m.state,
            started_ms: m.started.map(ms),
            duration_ms,
        })
    }

    pub fn visible_view(&self, grid: &Grid<Cell>, view: &[(i32, i32)], at_bottom: bool) -> Vec<BlockSpan> {
        let Some(&(first_row, first_line)) = view.first() else { return Vec::new() };
        if self.list.is_empty() {
            return Vec::new();
        }
        let top = -(grid.history_size() as i32);
        let bottom = grid.screen_lines() as i32 - 1;

        let row_above = |line: i32| first_row - (first_line - line);

        let mut found: Vec<(u32, i32, i32)> = Vec::new();
        fn note(found: &mut Vec<(u32, i32, i32)>, id: u32, row: i32) {
            match found.iter_mut().find(|f| f.0 == id) {
                Some(f) => {
                    f.1 = f.1.min(row);
                    f.2 = f.2.max(row);
                }
                None => found.push((id, row, row)),
            }
        }
        for &(row, line) in view {
            if line >= top
                && line <= bottom
                && let Some(id) = row_tag(grid, Line(line))
            {
                note(&mut found, id, row);
            }
        }

        let mut line = first_line - 1;
        let mut above: Option<u32> = None;
        while line >= top && (first_line - line <= FULL_SCAN_ABOVE || above.is_some()) {
            match (row_tag(grid, Line(line)), above) {
                (Some(id), None) => {
                    above = Some(id);
                    note(&mut found, id, row_above(line));
                }
                (Some(id), Some(a)) if id == a => note(&mut found, id, row_above(line)),
                (_, Some(_)) => break,
                (None, None) => {}
            }
            line -= 1;
        }
        if above.is_none() && line >= top {
            if at_bottom {
                let first_visible = found.iter().map(|f| f.0).min();
                let enclosing = self
                    .list
                    .iter()
                    .rev()
                    .find(|b| first_visible.is_none_or(|v| b.id < v) && b.state != BlockState::Editing);
                if let Some(b) = enclosing {
                    note(&mut found, b.id, FAR_ABOVE);
                }
            } else if let Some((id, a, b)) = self.deep_scan(grid, line) {
                note(&mut found, id, row_above(a));
                note(&mut found, id, row_above(b));
            }
        }
        let mut spans: Vec<BlockSpan> = found.into_iter().filter_map(|(id, a, b)| self.span(id, a, b)).collect();
        spans.sort_by_key(|s| s.prompt_row);
        spans
    }
}

pub fn row_tag(grid: &Grid<Cell>, line: Line) -> Option<u32> {
    row_tag_prefix(grid, line, grid.columns())
}

fn row_tag_prefix(grid: &Grid<Cell>, line: Line, cells: usize) -> Option<u32> {
    let row = &grid[line];
    (0..grid.columns().min(cells)).find_map(|c| {
        let cell = &row[Column(c)];
        cell.extra.as_ref()?;
        let link = cell.hyperlink()?;
        link.uri().strip_prefix(TAG_PREFIX)?.parse().ok()
    })
}

pub fn find_block(grid: &Grid<Cell>, id: u32) -> Option<(i32, i32)> {
    let top = -(grid.history_size() as i32);
    let bottom = grid.screen_lines() as i32 - 1;
    let mut last = None;
    let mut first = None;
    let mut l = bottom;
    while l >= top {
        let tag = row_tag(grid, Line(l));
        match tag {
            Some(t) if t == id => {
                last.get_or_insert(l);
                first = Some(l);
            }
            Some(t) if t < id => break,
            _ if last.is_some() => break,
            _ => {}
        }
        l -= 1;
    }
    Some((first?, last?))
}

pub fn output_text(grid: &Grid<Cell>, id: u32) -> Option<String> {
    let (_, last) = find_block(grid, id)?;
    let bottom = grid.screen_lines() as i32 - 1;
    let mut out = String::new();
    let mut l = last + 1;
    while l <= bottom && row_tag(grid, Line(l)).is_none() {
        let row = &grid[Line(l)];
        let mut text: String = (0..grid.columns())
            .map(|c| &row[Column(c)])
            .filter(|c| !c.flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER))
            .map(|c| if c.c == '\t' { ' ' } else { c.c })
            .collect();
        let wrapped = row[Column(grid.columns() - 1)].flags.contains(Flags::WRAPLINE);
        if !wrapped {
            text.truncate(text.trim_end().len());
            text.push('\n');
        }
        out.push_str(&text);
        l += 1;
    }
    let trimmed = out.trim_end_matches('\n').len();
    out.truncate(trimmed);
    Some(out)
}

pub fn block_range(grid: &Grid<Cell>, id: u32) -> Option<(i32, i32)> {
    let (first, last) = find_block(grid, id)?;
    let bottom = grid.screen_lines() as i32 - 1;
    let mut l = last + 1;
    while l <= bottom && row_tag(grid, Line(l)).is_none() {
        l += 1;
    }

    let mut end = l - 1;
    while end > last && (0..grid.columns()).all(|c| grid[Line(end)][Column(c)].c == ' ') {
        end -= 1;
    }
    Some((first, end))
}

pub fn jump_target(grid: &Grid<Cell>, top_line: i32, forward: bool) -> Option<i32> {
    let top = -(grid.history_size() as i32);
    let bottom = grid.screen_lines() as i32 - 1;

    let run_start = |mut l: i32| {
        let id = row_tag(grid, Line(l));
        while l > top && row_tag(grid, Line(l - 1)) == id {
            l -= 1;
        }
        l
    };
    if forward {
        let current = row_tag(grid, Line(top_line));
        let mut l = top_line + 1;
        while l <= bottom {
            if let Some(id) = row_tag(grid, Line(l))
                && Some(id) != current
            {
                return Some(l);
            }
            l += 1;
        }
        None
    } else {
        let mut l = top_line - 1;
        while l >= top {
            if row_tag(grid, Line(l)).is_some() {
                let start = run_start(l);
                if start < top_line {
                    return Some(start);
                }
            }
            l -= 1;
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle_and_prompt_redraw() {
        let mut b = Blocks::default();
        let inject = b.on_event(&OscEvent::PromptStart).unwrap();
        assert!(inject.contains("zhell:b/1"));

        b.on_event(&OscEvent::PromptStart);
        assert_eq!(b.list.len(), 1);
        b.on_event(&OscEvent::CommandText("ls".into()));
        assert_eq!(b.on_event(&OscEvent::OutputStart).as_deref(), Some(CLOSE_TAG));
        assert_eq!(b.get(1).unwrap().state, BlockState::Running);
        b.on_event(&OscEvent::CommandFinished { exit_code: Some(2) });
        assert_eq!(b.get(1).unwrap().state, BlockState::Done { exit: Some(2) });
        assert_eq!(b.take_finished().map(|m| m.id), Some(1));

        b.on_event(&OscEvent::CommandFinished { exit_code: Some(0) });
        assert!(b.take_finished().is_none());
        assert_eq!(b.get(1).unwrap().cmd.as_deref(), Some("ls"));

        assert!(b.on_event(&OscEvent::PromptStart).unwrap().contains("zhell:b/2"));
    }

    #[test]
    fn finish_without_running_is_ignored() {
        let mut b = Blocks::default();
        b.on_event(&OscEvent::PromptStart);
        b.on_event(&OscEvent::CommandFinished { exit_code: Some(0) });
        assert_eq!(b.get(1).unwrap().state, BlockState::Editing);
    }
}
