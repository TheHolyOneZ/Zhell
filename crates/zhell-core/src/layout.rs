use serde::{Deserialize, Serialize};
use zhell_proto::PaneId;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SplitDir {
    Horizontal,
    Vertical,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Node {
    Leaf(PaneId),
    Split { dir: SplitDir, ratio: f32, a: Box<Node>, b: Box<Node> },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Divider {
    pub rect: Rect,
    pub dir: SplitDir,

    pub path: u64,
    pub depth: u8,

    pub area: Rect,
}

impl Node {
    pub fn panes(&self) -> Vec<PaneId> {
        let mut out = Vec::new();
        self.collect(&mut out);
        out
    }

    fn collect(&self, out: &mut Vec<PaneId>) {
        match self {
            Node::Leaf(p) => out.push(*p),
            Node::Split { a, b, .. } => {
                a.collect(out);
                b.collect(out);
            }
        }
    }

    pub fn rects(&self, area: Rect, gap: f32) -> Vec<(PaneId, Rect)> {
        let mut out = Vec::new();
        self.layout(area, gap, &mut out, &mut Vec::new(), 0, 0);
        out
    }

    pub fn dividers(&self, area: Rect, gap: f32) -> Vec<Divider> {
        let mut out = Vec::new();
        self.layout(area, gap, &mut Vec::new(), &mut out, 0, 0);
        out
    }

    fn layout(
        &self,
        area: Rect,
        gap: f32,
        rects: &mut Vec<(PaneId, Rect)>,
        dividers: &mut Vec<Divider>,
        path: u64,
        depth: u8,
    ) {
        match self {
            Node::Leaf(p) => rects.push((*p, area)),
            Node::Split { dir, ratio, a, b } => {
                let (ra, rb, div) = split_rect(area, *dir, *ratio, gap);
                dividers.push(Divider { rect: div, dir: *dir, path, depth, area });
                a.layout(ra, gap, rects, dividers, path, depth + 1);
                b.layout(rb, gap, rects, dividers, path | (1 << depth), depth + 1);
            }
        }
    }

    pub fn split(&mut self, target: PaneId, dir: SplitDir, new: PaneId) -> bool {
        match self {
            Node::Leaf(p) if *p == target => {
                *self = Node::Split {
                    dir,
                    ratio: 0.5,
                    a: Box::new(Node::Leaf(target)),
                    b: Box::new(Node::Leaf(new)),
                };
                true
            }
            Node::Leaf(_) => false,
            Node::Split { a, b, .. } => a.split(target, dir, new) || b.split(target, dir, new),
        }
    }

    pub fn remove(self, target: PaneId) -> Option<Node> {
        match self {
            Node::Leaf(p) if p == target => None,
            leaf @ Node::Leaf(_) => Some(leaf),
            Node::Split { dir, ratio, a, b } => match (a.remove(target), b.remove(target)) {
                (Some(a), Some(b)) => Some(Node::Split { dir, ratio, a: Box::new(a), b: Box::new(b) }),
                (Some(only), None) | (None, Some(only)) => Some(only),
                (None, None) => None,
            },
        }
    }

    pub fn set_ratio(&mut self, path: u64, depth: u8, ratio: f32) {
        let mut node = self;
        for level in 0..depth {
            match node {
                Node::Split { a, b, .. } => {
                    node = if path & (1 << level) != 0 { b } else { a };
                }
                Node::Leaf(_) => return,
            }
        }
        if let Node::Split { ratio: r, .. } = node {
            *r = ratio.clamp(0.1, 0.9);
        }
    }
}

fn split_rect(area: Rect, dir: SplitDir, ratio: f32, gap: f32) -> (Rect, Rect, Rect) {
    match dir {
        SplitDir::Horizontal => {
            let wa = ((area.w - gap) * ratio).round();
            let a = Rect { w: wa, ..area };
            let div = Rect { x: area.x + wa, w: gap, ..area };
            let b = Rect { x: area.x + wa + gap, w: area.w - wa - gap, ..area };
            (a, b, div)
        }
        SplitDir::Vertical => {
            let ha = ((area.h - gap) * ratio).round();
            let a = Rect { h: ha, ..area };
            let div = Rect { y: area.y + ha, h: gap, ..area };
            let b = Rect { y: area.y + ha + gap, h: area.h - ha - gap, ..area };
            (a, b, div)
        }
    }
}

pub fn neighbor(rects: &[(PaneId, Rect)], from: PaneId, dir: Direction) -> Option<PaneId> {
    let (_, f) = *rects.iter().find(|(p, _)| *p == from)?;
    let overlap = |a0: f32, a1: f32, b0: f32, b1: f32| (a1.min(b1) - a0.max(b0)).max(0.0);
    rects
        .iter()
        .filter(|(p, _)| *p != from)
        .filter_map(|(p, r)| {
            let (beside, dist, ov) = match dir {
                Direction::Left => (r.x + r.w <= f.x + 0.5, f.x - (r.x + r.w), overlap(r.y, r.y + r.h, f.y, f.y + f.h)),
                Direction::Right => (r.x >= f.x + f.w - 0.5, r.x - (f.x + f.w), overlap(r.y, r.y + r.h, f.y, f.y + f.h)),
                Direction::Up => (r.y + r.h <= f.y + 0.5, f.y - (r.y + r.h), overlap(r.x, r.x + r.w, f.x, f.x + f.w)),
                Direction::Down => (r.y >= f.y + f.h - 0.5, r.y - (f.y + f.h), overlap(r.x, r.x + r.w, f.x, f.x + f.w)),
            };
            (beside && ov > 0.0).then_some((*p, dist, ov))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1).then(b.2.total_cmp(&a.2)))
        .map(|(p, _, _)| p)
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Tab {
    pub root: Node,
    pub focus: PaneId,

    pub zoomed: bool,
}

impl Tab {
    pub fn new(pane: PaneId) -> Self {
        Self { root: Node::Leaf(pane), focus: pane, zoomed: false }
    }

    pub fn rects(&self, area: Rect, gap: f32) -> Vec<(PaneId, Rect)> {
        if self.zoomed {
            vec![(self.focus, area)]
        } else {
            self.root.rects(area, gap)
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Layout {
    pub tabs: Vec<Tab>,
    pub active: usize,
}

impl Layout {
    pub fn active_tab(&self) -> Option<&Tab> {
        self.tabs.get(self.active)
    }

    pub fn active_tab_mut(&mut self) -> Option<&mut Tab> {
        self.tabs.get_mut(self.active)
    }

    pub fn focused(&self) -> Option<PaneId> {
        self.active_tab().map(|t| t.focus)
    }

    pub fn add_tab(&mut self, pane: PaneId) {
        let at = (self.active + 1).min(self.tabs.len());
        self.tabs.insert(at, Tab::new(pane));
        self.active = at;
    }

    pub fn split_focused(&mut self, dir: SplitDir, new: PaneId) -> bool {
        let Some(tab) = self.active_tab_mut() else { return false };
        let target = tab.focus;
        if tab.root.split(target, dir, new) {
            tab.focus = new;
            tab.zoomed = false;
            true
        } else {
            false
        }
    }

    pub fn remove_pane(&mut self, pane: PaneId) -> bool {
        let Some(ti) = self.tabs.iter().position(|t| t.root.panes().contains(&pane)) else {
            return self.tabs.is_empty();
        };
        let tab = &mut self.tabs[ti];
        let root = std::mem::replace(&mut tab.root, Node::Leaf(pane));
        match root.remove(pane) {
            Some(rest) => {
                if tab.focus == pane {
                    tab.focus = rest.panes()[0];
                    tab.zoomed = false;
                }
                tab.root = rest;
            }
            None => {
                self.tabs.remove(ti);
                if self.active >= self.tabs.len() || ti < self.active {
                    self.active = self.active.saturating_sub(1);
                }
            }
        }
        self.tabs.is_empty()
    }

    pub fn tab_of(&self, pane: PaneId) -> Option<usize> {
        self.tabs.iter().position(|t| t.root.panes().contains(&pane))
    }

    pub fn move_tab(&mut self, from: usize, to: usize) {
        if from >= self.tabs.len() || to >= self.tabs.len() || from == to {
            return;
        }
        let active_tab = self.active;
        let t = self.tabs.remove(from);
        self.tabs.insert(to, t);
        self.active = if active_tab == from {
            to
        } else if from < active_tab && to >= active_tab {
            active_tab - 1
        } else if from > active_tab && to <= active_tab {
            active_tab + 1
        } else {
            active_tab
        };
    }

    pub fn select_tab(&mut self, index: usize) {
        if index < self.tabs.len() {
            self.active = index;
        }
    }

    pub fn cycle_tab(&mut self, forward: bool) {
        let n = self.tabs.len();
        if n > 0 {
            self.active = if forward { (self.active + 1) % n } else { (self.active + n - 1) % n };
        }
    }

    pub fn move_focus(&mut self, area: Rect, gap: f32, dir: Direction) {
        let Some(tab) = self.active_tab_mut() else { return };
        if tab.zoomed {
            return;
        }
        let rects = tab.root.rects(area, gap);
        if let Some(p) = neighbor(&rects, tab.focus, dir) {
            tab.focus = p;
        }
    }

    pub fn all_panes(&self) -> Vec<PaneId> {
        self.tabs.iter().flat_map(|t| t.root.panes()).collect()
    }

    pub fn resize_focused(&mut self, area: Rect, gap: f32, dir: Direction, step: f32) -> bool {
        let Some(tab) = self.active_tab_mut() else { return false };
        if tab.zoomed {
            return false;
        }
        let rects = tab.root.rects(area, gap);
        let Some(&(_, f)) = rects.iter().find(|(p, _)| *p == tab.focus) else { return false };
        let overlaps = |a0: f32, a1: f32, b0: f32, b1: f32| a1.min(b1) - a0.max(b0) > 0.0;
        let near = |a: f32, b: f32| (a - b).abs() <= gap + 1.0;
        let dividers = tab.root.dividers(area, gap);
        let horizontal = matches!(dir, Direction::Left | Direction::Right);

        let edge = |d: &&Divider| match (horizontal, d.dir) {
            (true, SplitDir::Horizontal) if overlaps(d.rect.y, d.rect.y + d.rect.h, f.y, f.y + f.h) => {
                if near(d.rect.x, f.x + f.w) {
                    Some(true)
                } else if near(d.rect.x + d.rect.w, f.x) {
                    Some(false)
                } else {
                    None
                }
            }
            (false, SplitDir::Vertical) if overlaps(d.rect.x, d.rect.x + d.rect.w, f.x, f.x + f.w) => {
                if near(d.rect.y, f.y + f.h) {
                    Some(true)
                } else if near(d.rect.y + d.rect.h, f.y) {
                    Some(false)
                } else {
                    None
                }
            }
            _ => None,
        };
        let forward = matches!(dir, Direction::Right | Direction::Down);
        let pick = dividers.iter().find(|d| edge(d) == Some(forward)).or_else(|| dividers.iter().find(|d| edge(d) == Some(!forward)));
        let Some(d) = pick.copied() else { return false };
        let delta = if forward { step } else { -step };
        let ratio = if horizontal {
            (d.rect.x + delta - d.area.x) / (d.area.w - gap).max(1.0)
        } else {
            (d.rect.y + delta - d.area.y) / (d.area.h - gap).max(1.0)
        };
        tab.root.set_ratio(d.path, d.depth, ratio);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyboard_resize_moves_the_nearest_border() {
        let area = Rect { x: 0.0, y: 0.0, w: 1000.0, h: 600.0 };
        let mut l = Layout::default();
        l.add_tab(PaneId(1));
        assert!(!l.resize_focused(area, 2.0, Direction::Right, 50.0), "nothing to resize");
        l.split_focused(SplitDir::Horizontal, PaneId(2));

        assert_eq!(l.focused(), Some(PaneId(2)));
        assert!(l.resize_focused(area, 2.0, Direction::Right, 100.0));
        let r = l.active_tab().unwrap().rects(area, 2.0);
        assert!((r[0].1.w - 599.0).abs() < 2.0, "{r:?}");

        l.resize_focused(area, 2.0, Direction::Left, 100.0);
        let r = l.active_tab().unwrap().rects(area, 2.0);
        assert!((r[0].1.w - 499.0).abs() < 2.0, "{r:?}");

        assert!(!l.resize_focused(area, 2.0, Direction::Down, 10.0));
    }

    const AREA: Rect = Rect { x: 0.0, y: 0.0, w: 101.0, h: 51.0 };
    fn p(n: u64) -> PaneId {
        PaneId(n)
    }

    #[test]
    fn split_and_rects_cover_area_with_gap() {
        let mut l = Layout::default();
        l.add_tab(p(1));
        assert!(l.split_focused(SplitDir::Horizontal, p(2)));
        assert!(l.split_focused(SplitDir::Vertical, p(3)));
        let rects = l.active_tab().unwrap().rects(AREA, 1.0);
        assert_eq!(rects.len(), 3);
        let r1 = rects[0].1;
        let r2 = rects[1].1;
        let r3 = rects[2].1;
        assert_eq!((r1.x, r1.w, r1.h), (0.0, 50.0, 51.0));
        assert_eq!((r2.x, r2.y, r2.w, r2.h), (51.0, 0.0, 50.0, 25.0));
        assert_eq!((r3.x, r3.y, r3.h), (51.0, 26.0, 25.0));
        assert_eq!(l.focused(), Some(p(3)));
    }

    #[test]
    fn focus_moves_by_geometry() {
        let mut l = Layout::default();
        l.add_tab(p(1));
        l.split_focused(SplitDir::Horizontal, p(2));
        l.split_focused(SplitDir::Vertical, p(3));
        l.move_focus(AREA, 1.0, Direction::Up);
        assert_eq!(l.focused(), Some(p(2)));
        l.move_focus(AREA, 1.0, Direction::Left);
        assert_eq!(l.focused(), Some(p(1)));
        l.move_focus(AREA, 1.0, Direction::Left);
        assert_eq!(l.focused(), Some(p(1)), "no pane to the left");
        l.move_focus(AREA, 1.0, Direction::Right);
        assert!(matches!(l.focused(), Some(PaneId(2 | 3))));
    }

    #[test]
    fn removing_panes_collapses_splits_and_tabs() {
        let mut l = Layout::default();
        l.add_tab(p(1));
        l.split_focused(SplitDir::Horizontal, p(2));
        l.add_tab(p(3));
        assert_eq!(l.active, 1);
        assert!(!l.remove_pane(p(2)));
        assert_eq!(l.tabs[0].root, Node::Leaf(p(1)));
        assert!(!l.remove_pane(p(3)));
        assert_eq!((l.tabs.len(), l.active), (1, 0));
        assert!(l.remove_pane(p(1)));
    }

    #[test]
    fn focus_falls_back_when_focused_pane_closes() {
        let mut l = Layout::default();
        l.add_tab(p(1));
        l.split_focused(SplitDir::Horizontal, p(2));
        l.remove_pane(p(2));
        assert_eq!(l.focused(), Some(p(1)));
    }

    #[test]
    fn closing_a_tab_before_the_active_keeps_the_same_tab_active() {
        let mut l = Layout::default();
        l.add_tab(p(1));
        l.add_tab(p(2));
        l.add_tab(p(3));
        assert_eq!(l.active, 2);
        l.remove_pane(p(1));
        assert_eq!(l.focused(), Some(p(3)));
    }

    #[test]
    fn moving_tabs_keeps_the_active_one() {
        let mut l = Layout::default();
        for i in 1..=4 {
            l.add_tab(p(i));
        }
        l.select_tab(1);
        l.move_tab(3, 0);
        assert_eq!(l.tabs.iter().map(|t| t.focus.0).collect::<Vec<_>>(), vec![4, 1, 2, 3]);
        assert_eq!(l.focused(), Some(p(2)));
        l.move_tab(2, 3);
        assert_eq!(l.focused(), Some(p(2)));
        assert_eq!(l.active, 3);
    }

    #[test]
    fn divider_drag_sets_ratio() {
        let mut l = Layout::default();
        l.add_tab(p(1));
        l.split_focused(SplitDir::Horizontal, p(2));
        l.split_focused(SplitDir::Vertical, p(3));
        let root = &mut l.tabs[0].root;
        let divs = root.dividers(AREA, 1.0);
        assert_eq!(divs.len(), 2);
        let inner = divs[1];
        root.set_ratio(inner.path, inner.depth, 0.25);
        match root {
            Node::Split { b, .. } => assert!(matches!(**b, Node::Split { ratio, .. } if ratio == 0.25)),
            _ => panic!(),
        }
    }
}
