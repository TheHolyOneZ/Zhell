use std::collections::{BTreeMap, BTreeSet};

use alacritty_terminal::grid::{Dimensions, Grid};
use alacritty_terminal::index::Line;
use alacritty_terminal::term::cell::Cell;

use crate::blocks::{find_block, row_tag};

pub const KEEP_LINES: i32 = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowSrc {
    Line(i32),

    Fold { block: u32, hidden: u32 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hidden {
    pub first: i32,
    pub last: i32,
    pub block: u32,
}

#[derive(Default)]
pub struct Folds {
    pub blocks: BTreeSet<u32>,

    cache: BTreeMap<u32, (usize, i64, i64)>,
}

impl Folds {
    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    pub fn set(&mut self, block: u32, folded: bool) {
        if folded {
            self.blocks.insert(block);
        } else {
            self.blocks.remove(&block);
            self.cache.remove(&block);
        }
    }

    pub fn hidden(&mut self, grid: &Grid<Cell>) -> Vec<Hidden> {
        let hist = grid.history_size() as i64;
        let cols = grid.columns();

        let bottom = (grid.cursor.point.line.0 - 1).min(grid.screen_lines() as i32 - 1);
        let mut out = Vec::new();
        let mut gone = Vec::new();
        for &block in &self.blocks {
            let cached = self.cache.get(&block).copied().filter(|(c, ..)| *c == cols).and_then(|(_, a, b)| {
                let (first, last) = ((a - hist) as i32, (b - hist) as i32);
                let tag_line = first - KEEP_LINES - 1;
                (tag_line >= -(hist as i32) && row_tag(grid, Line(tag_line)) == Some(block)).then_some((first, last))
            });
            let range = match cached {
                Some((first, _)) => Some((first, output_end(grid, first, bottom))),
                None => find_block(grid, block).map(|(_, last_tag)| {
                    let start = last_tag + 1;
                    (start + KEEP_LINES, output_end(grid, start, bottom))
                }),
            };
            match range {
                Some((first, last)) if last > first => {
                    self.cache.insert(block, (cols, first as i64 + hist, last as i64 + hist));
                    out.push(Hidden { first, last, block });
                }
                Some(_) => {}
                None => gone.push(block),
            }
        }

        for b in gone {
            self.blocks.remove(&b);
            self.cache.remove(&b);
        }
        out.sort_by_key(|h| h.first);
        out
    }
}

fn output_end(grid: &Grid<Cell>, from: i32, bottom: i32) -> i32 {
    let mut l = from;
    while l <= bottom && row_tag(grid, Line(l)).is_none() {
        l += 1;
    }
    l - 1
}

pub fn row_map(rows: usize, bottom_line: i32, top_line: i32, hidden: &[Hidden]) -> Vec<RowSrc> {
    let mut out = Vec::with_capacity(rows);
    let mut l = bottom_line;
    while out.len() < rows && l >= top_line {
        if let Some(h) = hidden.iter().find(|h| l >= h.first && l <= h.last) {
            out.push(RowSrc::Fold { block: h.block, hidden: (h.last - h.first + 1) as u32 });
            l = h.first - 1;
            continue;
        }
        out.push(RowSrc::Line(l));
        l -= 1;
    }
    if out.len() == rows {
        out.reverse();
        return out;
    }

    out.clear();
    let mut l = top_line;
    while out.len() < rows {
        if let Some(h) = hidden.iter().find(|h| l >= h.first && l <= h.last) {
            out.push(RowSrc::Fold { block: h.block, hidden: (h.last - h.first + 1) as u32 });
            l = h.last + 1;
            continue;
        }
        out.push(RowSrc::Line(l));
        l += 1;
    }
    out
}

pub fn row_of(map: &[RowSrc], hidden: &[Hidden], line: i32) -> Option<usize> {
    let target = hidden.iter().find(|h| line >= h.first && line <= h.last).map(|h| h.block);
    map.iter().position(|r| match (r, target) {
        (RowSrc::Fold { block, .. }, Some(t)) => *block == t,
        (RowSrc::Line(l), None) => *l == line,
        _ => false,
    })
}

pub fn scroll_anchor(bottom: i32, delta: i32, top_line: i32, max_bottom: i32, hidden: &[Hidden]) -> i32 {
    let mut l = bottom;
    let step = if delta > 0 { -1 } else { 1 };
    for _ in 0..delta.unsigned_abs() {
        let mut next = l + step;
        if let Some(h) = hidden.iter().find(|h| next >= h.first && next <= h.last) {
            let inside = l >= h.first && l <= h.last;
            next = match (inside, step < 0) {
                (false, _) => h.first,
                (true, true) => h.first - 1,
                (true, false) => h.last + 1,
            };
        }
        if next < top_line || next > max_bottom {
            break;
        }
        l = next;
    }
    l
}

#[cfg(test)]
mod tests {
    use super::*;

    const H: Hidden = Hidden { first: 10, last: 99, block: 7 };

    #[test]
    fn map_replaces_hidden_lines_with_one_row() {
        let map = row_map(5, 101, -1000, &[H]);
        assert_eq!(
            map,
            vec![RowSrc::Line(8), RowSrc::Line(9), RowSrc::Fold { block: 7, hidden: 90 }, RowSrc::Line(100), RowSrc::Line(101)]
        );
        assert_eq!(row_of(&map, &[H], 50), Some(2));
        assert_eq!(row_of(&map, &[H], 9), Some(1));
        assert_eq!(row_of(&map, &[H], 200), None);
    }

    #[test]
    fn short_history_starts_at_the_top_and_skips_folds() {
        let map = row_map(6, 50, 5, &[H]);
        assert_eq!(
            map,
            vec![
                RowSrc::Line(5),
                RowSrc::Line(6),
                RowSrc::Line(7),
                RowSrc::Line(8),
                RowSrc::Line(9),
                RowSrc::Fold { block: 7, hidden: 90 },
            ]
        );
        let map = row_map(8, 50, 5, &[H]);
        assert_eq!(map[6..], [RowSrc::Line(100), RowSrc::Line(101)]);
    }

    #[test]
    fn short_grids_pad_downwards() {
        let map = row_map(4, 1, 0, &[]);
        assert_eq!(map, vec![RowSrc::Line(0), RowSrc::Line(1), RowSrc::Line(2), RowSrc::Line(3)]);
    }

    #[test]
    fn scrolling_steps_over_a_fold() {
        assert_eq!(scroll_anchor(101, 3, -1000, 101, &[H]), 9);

        assert_eq!(scroll_anchor(9, -2, -1000, 101, &[H]), 100);

        assert_eq!(scroll_anchor(100, -5, -1000, 101, &[H]), 101);
    }
}
