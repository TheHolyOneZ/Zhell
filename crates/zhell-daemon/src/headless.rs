use alacritty_terminal::event::VoidListener;
use alacritty_terminal::term::{Config, Term};
use alacritty_terminal::vte::ansi::Processor;
use zhell_proto::{FrameDiff, PaneId, TermSize};

use crate::pane::{build_frame, grid_size};

pub struct Headless {
    term: Term<VoidListener>,
    parser: Processor,
}

impl Headless {
    pub fn new(cols: u16, rows: u16) -> Self {
        let size = TermSize { cols, rows, cell_width: 8, cell_height: 16 };
        let config = Config { scrolling_history: 1000, ..Config::default() };
        Self { term: Term::new(config, &grid_size(size), VoidListener), parser: Processor::new() }
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.term, bytes);
    }

    pub fn frame(&mut self) -> FrameDiff {
        build_frame(&mut self.term, PaneId(0), 0, true, None, None, None, None)
    }
}
