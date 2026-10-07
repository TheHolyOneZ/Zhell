use zhell_proto::{BlockSpan, Cell, CursorShape, CursorState, FindState, FoldRow, FrameDiff, ImageRow, SelectionSpan};

#[derive(Clone)]
pub struct Mirror {
    pub cols: u16,
    pub rows: u16,
    pub lines: Vec<Vec<Cell>>,
    pub cursor: CursorState,
    pub modes: u32,
    pub display_offset: u32,

    pub history_len: u32,
    pub selection: Option<SelectionSpan>,
    pub blocks: Vec<BlockSpan>,
    pub images: Vec<ImageRow>,
    pub find: Option<FindState>,
    pub folds: Vec<FoldRow>,

    pub last_seq: Option<u64>,
}

impl Mirror {
    pub fn new() -> Self {
        Self {
            cols: 0,
            rows: 0,
            lines: Vec::new(),
            cursor: CursorState { row: 0, col: 0, shape: CursorShape::Block, blinking: false },
            modes: 0,
            display_offset: 0,
            history_len: 0,
            selection: None,
            blocks: Vec::new(),
            images: Vec::new(),
            find: None,
            folds: Vec::new(),
            last_seq: None,
        }
    }

    pub fn apply(&mut self, f: FrameDiff) {
        if f.full || f.cols != self.cols || f.rows != self.rows {
            self.lines = vec![vec![Cell::default(); f.cols as usize]; f.rows as usize];
        }
        self.cols = f.cols;
        self.rows = f.rows;
        for l in f.lines {
            if let Some(dst) = self.lines.get_mut(l.row as usize) {
                *dst = l.cells;
            }
        }
        self.cursor = f.cursor;
        self.modes = f.modes;
        self.display_offset = f.display_offset;
        self.history_len = f.history_len;
        self.selection = f.selection;
        self.blocks = f.blocks;
        self.images = f.images;
        self.find = f.find;
        self.folds = f.folds;
        self.last_seq = Some(f.seq);
    }
}

impl Mirror {
    pub fn block_end(&self, i: usize) -> i32 {
        match self.blocks.get(i + 1) {
            Some(next) => next.prompt_row - 1,
            None if self.display_offset == 0 => self.cursor.row as i32,
            None => self.rows as i32 - 1,
        }
    }

    pub fn block_at(&self, row: i32) -> Option<usize> {
        (0..self.blocks.len()).rev().find(|&i| self.blocks[i].prompt_row <= row && row <= self.block_end(i))
    }
}
