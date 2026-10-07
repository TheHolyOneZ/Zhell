use zhell_core::layout::Rect;
use zhell_proto::{Cell, CursorShape, flags};
use zhell_render::{CellMetrics, GlyphStyle, Renderer};

use crate::mirror::Mirror;
use crate::theme::{self, Rgba, Theme};

#[derive(Clone)]
pub struct PaneStyle {
    pub origin: (f32, f32),

    pub area: Rect,
    pub scale: f32,

    pub focused: bool,

    pub cursor_visible: bool,

    pub dim: bool,

    pub cursor_offset: (f32, f32),

    pub ligatures: bool,

    pub pane_key: u64,

    pub redact: bool,

    pub link: Vec<(u16, u16)>,

    pub bg_alpha: f32,
}

pub fn cell_colors(theme: &Theme, cell: &Cell) -> (Rgba, Rgba) {
    let mut fg = theme.resolve(cell.fg);
    let mut bg = theme.resolve(cell.bg);
    if cell.flags & flags::DIM != 0 {
        fg = theme::dim(fg);
    }
    if cell.flags & flags::INVERSE != 0 {
        std::mem::swap(&mut fg, &mut bg);
    }
    (fg, bg)
}

fn glyph_text(cell: &Cell, f: impl FnOnce(&str)) {
    if cell.zerowidth.is_empty() {
        let mut buf = [0u8; 4];
        f(cell.ch.encode_utf8(&mut buf));
    } else {
        let mut s = String::from(cell.ch);
        s.extend(&cell.zerowidth);
        f(&s);
    }
}

pub fn images(r: &mut Renderer, mirror: &Mirror, images: &std::collections::HashMap<u32, crate::view::ImagePixels>, style: &PaneStyle) {
    let m = r.cell_metrics();
    let (ox, oy) = style.origin;
    for ir in &mirror.images {
        let Some(img) = images.get(&ir.id) else { continue };
        let sy = ir.image_row as f32 * m.height;
        if sy >= img.height as f32 {
            continue;
        }
        let sh = (img.height as f32 - sy).min(m.height);
        let key = (style.pane_key << 32) | ir.id as u64;
        r.image(
            key,
            img.width,
            img.height,
            &img.rgba,
            [0.0, sy, img.width as f32, sh],
            [ox + ir.col as f32 * m.width, oy + ir.row as f32 * m.height, img.width as f32, sh],
        );
    }
}

pub fn pane(r: &mut Renderer, theme: &Theme, mirror: &Mirror, style: &PaneStyle) {
    let (ox, oy) = style.origin;
    let (area, scale) = (style.area, style.scale);
    let m = r.cell_metrics();
    let cur = mirror.cursor;
    let show_cursor = cur.shape != CursorShape::Hidden && mirror.display_offset == 0 && style.cursor_visible;
    let moving = style.cursor_offset.0.abs() > 0.5 || style.cursor_offset.1.abs() > 0.5;

    let block_cursor = show_cursor && style.focused && cur.shape == CursorShape::Block && !moving;
    let thin = scale.round().max(1.0);
    let selection = mirror.selection;
    let ligatures = style.ligatures;

    for (row, line) in mirror.lines.iter().enumerate() {
        let y = oy + row as f32 * m.height;

        if mirror.folds.iter().any(|f| f.row as usize == row) {
            let (f, b) = (theme.foreground, theme.background);
            let bar = [b[0] + (f[0] - b[0]) * 0.08, b[1] + (f[1] - b[1]) * 0.08, b[2] + (f[2] - b[2]) * 0.08, 1.0];
            let w = (m.width * 40.0).min(area.x + area.w - ox - (ox - area.x));
            r.rounded_rect(ox, y + scale, w, m.height - 2.0 * scale, theme::RADIUS * scale, bar);
        }

        for (col, cell) in line.iter().enumerate() {
            let (_, bg) = cell_colors(theme, cell);
            let x = ox + col as f32 * m.width;
            let w = if cell.flags & flags::WIDE_CHAR != 0 { m.width * 2.0 } else { m.width };
            if block_cursor && row == cur.row as usize && col == cur.col as usize {
                r.rect(x, y, w, m.height, theme.cursor);
            } else if bg != theme.background {
                r.rect(x, y, w, m.height, [bg[0], bg[1], bg[2], bg[3] * style.bg_alpha]);
            }
            if selection.is_some_and(|s| s.contains(row as i32, col as u16)) {
                r.rect(x, y, w, m.height, theme.selection);
            }
            if let Some(f) = &mirror.find {
                let (r_, c_) = (row as i32, col as u16);
                if f.current.is_some_and(|s| s.contains(r_, c_)) {
                    let a = theme.palette[3];
                    r.rect(x, y, w, m.height, [a[0], a[1], a[2], 0.55]);
                } else if f.matches.iter().any(|s| s.contains(r_, c_)) {
                    let a = theme.accent;
                    r.rect(x, y, w, m.height, [a[0], a[1], a[2], 0.35]);
                }
            }
        }

        let mut word = String::new();
        let mut word_start = 0usize;
        let mut word_style: Option<(GlyphStyle, Rgba)> = None;
        let flush = |r: &mut Renderer, word: &mut String, start: usize, style: Option<(GlyphStyle, Rgba)>| {
            if let Some((gs, fg)) = style
                && !word.is_empty()
            {
                let x = ox + start as f32 * m.width;
                if style.is_some() && ligatures && word.chars().count() > 1 {
                    r.word(x, y, word, gs, fg);
                } else {
                    let mut cx = x;
                    let mut buf = [0u8; 4];
                    for ch in word.chars() {
                        r.glyph(cx, y, ch.encode_utf8(&mut buf), gs, fg);
                        cx += m.width;
                    }
                }
            }
            word.clear();
        };
        for (col, cell) in line.iter().enumerate() {
            if cell.flags & (flags::WIDE_CHAR_SPACER | flags::LEADING_WIDE_CHAR_SPACER) != 0 {
                continue;
            }
            let (mut fg, bg) = cell_colors(theme, cell);
            let x = ox + col as f32 * m.width;
            if block_cursor && row == cur.row as usize && col == cur.col as usize {
                fg = if bg == theme.background { theme.background } else { bg };
            }
            let visible = !matches!(cell.ch, ' ' | '\0' | '\t') && cell.flags & flags::HIDDEN == 0;
            let gs = GlyphStyle { bold: cell.flags & flags::BOLD != 0, italic: cell.flags & flags::ITALIC != 0 };
            let simple = visible
                && cell.zerowidth.is_empty()
                && cell.flags & flags::WIDE_CHAR == 0
                && !zhell_render::is_builtin_glyph(cell.ch);
            if simple && word_style == Some((gs, fg)) && word_start + word.chars().count() == col {
                word.push(cell.ch);
            } else {
                flush(r, &mut word, word_start, word_style);
                word_style = None;
                if simple {
                    word.push(cell.ch);
                    word_start = col;
                    word_style = Some((gs, fg));
                } else if visible {
                    glyph_text(cell, |t| r.glyph(x, y, t, gs, fg));
                }
            }
            decorations(r, theme, cell, x, y, m, fg, thin, scale);
        }
        flush(r, &mut word, word_start, word_style);
    }

    if show_cursor && !block_cursor {
        let x = ox + cur.col as f32 * m.width + style.cursor_offset.0;
        let y = oy + cur.row as f32 * m.height + style.cursor_offset.1;
        let c = theme.cursor;
        let t = (2.0 * scale).round();
        match (style.focused, cur.shape) {
            (true, CursorShape::Block) => r.rounded_rect(x, y, m.width, m.height, scale, [c[0], c[1], c[2], 0.9]),
            (true, CursorShape::Beam) => r.rect(x, y, t, m.height, c),
            (true, CursorShape::Underline) => r.rect(x, y + m.height - t, m.width, t, c),
            _ => {
                r.rect(x, y, m.width, thin, c);
                r.rect(x, y + m.height - thin, m.width, thin, c);
                r.rect(x, y, thin, m.height, c);
                r.rect(x + m.width - thin, y, thin, m.height, c);
            }
        }
    }

    if style.redact {
        redact_secrets(r, theme, mirror, ox, oy, scale);
    }

    let thick = (2.0 * scale).round().max(1.0);
    for &(row, col) in &style.link {
        let x = ox + col as f32 * m.width;
        let y = oy + row as f32 * m.height + m.height - thick;
        r.rect(x, y, m.width, thick, theme.accent);
    }

    if style.dim {
        let b = theme.background;
        r.rect(area.x, area.y, area.w, area.h, [b[0], b[1], b[2], 0.35]);
    }
}

#[allow(clippy::too_many_arguments)]
fn decorations(
    r: &mut Renderer,
    theme: &Theme,
    cell: &Cell,
    x: f32,
    y: f32,
    m: CellMetrics,
    fg: Rgba,
    thin: f32,
    scale: f32,
) {
    let w = if cell.flags & flags::WIDE_CHAR != 0 { m.width * 2.0 } else { m.width };
    let underline = flags::UNDERLINE
        | flags::DOUBLE_UNDERLINE
        | flags::UNDERCURL
        | flags::DOTTED_UNDERLINE
        | flags::DASHED_UNDERLINE;
    if cell.flags & underline != 0 || cell.hyperlink.is_some() {
        let uc = cell.underline_color.map(|c| theme.resolve(c)).unwrap_or(fg);
        let uy = (y + m.baseline + 2.0 * scale).min(y + m.height - thin);
        if cell.flags & flags::UNDERCURL != 0 {
            let amp = (scale * 1.5).round().max(1.0);
            let period = (m.width / 2.0).max(4.0);
            let mut px = 0.0;
            while px < w {
                let phase = ((x + px) / period * std::f32::consts::TAU).sin();
                r.rect(x + px, uy - amp + phase * amp, 1.0, thin, uc);
                px += 1.0;
            }
        } else if cell.flags & flags::DOTTED_UNDERLINE != 0 {
            let mut px = 0.0;
            while px < w {
                r.rect(x + px, uy, thin, thin, uc);
                px += thin * 2.0;
            }
        } else if cell.flags & flags::DASHED_UNDERLINE != 0 {
            let dash = (m.width / 3.0).round().max(2.0);
            r.rect(x, uy, dash, thin, uc);
            r.rect(x + dash * 2.0, uy, (w - dash * 2.0).max(0.0), thin, uc);
        } else {
            r.rect(x, uy, w, thin, uc);
            if cell.flags & flags::DOUBLE_UNDERLINE != 0 {
                r.rect(x, (uy + 2.0 * thin).min(y + m.height - thin), w, thin, uc);
            }
        }
    }
    if cell.flags & flags::STRIKEOUT != 0 {
        r.rect(x, y + (m.height / 2.0).round(), w, thin, fg);
    }
}

pub fn label(r: &mut Renderer, text: &str, x: f32, y: f32, max_w: f32, color: Rgba, bold: bool) -> f32 {
    let m = r.cell_metrics();
    let mut cx = x;
    let chars: Vec<char> = text.chars().collect();
    let fits = (max_w / m.width).floor() as usize;
    let truncated = chars.len() > fits;
    let shown = if truncated { fits.saturating_sub(1) } else { chars.len() };
    let style = GlyphStyle { bold, italic: false };
    let mut buf = [0u8; 4];
    for ch in chars.iter().take(shown) {
        r.glyph(cx, y, ch.encode_utf8(&mut buf), style, color);
        cx += m.width;
    }
    if truncated && fits > 0 {
        r.glyph(cx, y, "…", style, color);
        cx += m.width;
    }
    cx
}

pub struct TabInfo {
    pub label: String,
    pub active: bool,

    pub ports: Vec<u16>,

    pub recording: bool,
}

pub fn preedit(r: &mut Renderer, theme: &Theme, text: &str, x: f32, y: f32, scale: f32) {
    use unicode_width::UnicodeWidthChar;
    let m = r.cell_metrics();
    let cols: usize = text.chars().map(|c| c.width().unwrap_or(0)).sum();
    let w = cols.max(1) as f32 * m.width;
    let f = theme.foreground;
    let b = theme.background;
    r.rect(x, y, w, m.height, [b[0] + (f[0] - b[0]) * 0.12, b[1] + (f[1] - b[1]) * 0.12, b[2] + (f[2] - b[2]) * 0.12, 1.0]);
    let mut cx = x;
    let mut buf = [0u8; 4];
    for ch in text.chars() {
        r.glyph(cx, y, ch.encode_utf8(&mut buf), GlyphStyle::default(), theme.foreground);
        cx += ch.width().unwrap_or(0) as f32 * m.width;
    }
    let t = scale.round().max(1.0);
    r.rect(x, y + m.height - t, w, t, theme.accent);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockAction {
    ToggleFold,
    CopyCommand,
    CopyOutput,
    SaveOutput,
    Rerun,
}

#[derive(Clone, Copy, Debug)]
pub struct BlockButton {
    pub rect: Rect,
    pub block: u32,
    pub action: BlockAction,
}

fn format_duration(ms: u64) -> String {
    match ms {
        0..=999 => format!("{ms} ms"),
        1000..=59_999 => format!("{:.1} s", ms as f64 / 1000.0),
        60_000..=3_599_999 => format!("{} m {} s", ms / 60_000, (ms / 1000) % 60),
        _ => format!("{} h {} m", ms / 3_600_000, (ms / 60_000) % 60),
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

fn block_status(theme: &Theme, b: &zhell_proto::BlockSpan) -> Option<(String, Rgba)> {
    use zhell_proto::BlockState;
    match b.state {
        BlockState::Editing => None,
        BlockState::Running => {
            let elapsed = b.started_ms.map(|s| now_ms().saturating_sub(s)).unwrap_or(0);
            Some((format!("● {}", format_duration(elapsed)), theme.accent))
        }
        BlockState::Done { exit } => {
            let d = b.duration_ms.map(format_duration).unwrap_or_default();
            match exit {
                Some(0) => Some((format!("✓ {d}"), theme.palette[2])),
                Some(code) => Some((format!("✗ {code} · {d}"), theme.palette[1])),
                None => Some((format!("· {d}"), theme::dim(theme.foreground))),
            }
        }
    }
}

pub fn blocks(
    r: &mut Renderer,
    theme: &Theme,
    mirror: &Mirror,
    style: &PaneStyle,
    hovered: Option<u32>,
) -> Vec<BlockButton> {
    use zhell_proto::BlockState;
    let m = r.cell_metrics();
    let (ox, oy) = style.origin;
    let area = style.area;
    let scale = style.scale;
    let rows = mirror.rows as i32;
    let mut buttons = Vec::new();
    if mirror.modes & zhell_proto::mode::ALT_SCREEN != 0 || mirror.blocks.is_empty() {
        return buttons;
    }
    let bar_w = (3.0 * scale).round().max(2.0);
    let bar_x = (ox - (ox - area.x) / 2.0 - bar_w / 2.0).round();
    let right = area.x + area.w - (ox - area.x);
    let fg = theme.foreground;
    let bg = theme.background;
    let mix = |t: f32| [bg[0] + (fg[0] - bg[0]) * t, bg[1] + (fg[1] - bg[1]) * t, bg[2] + (fg[2] - bg[2]) * t, 1.0];
    let row_y = |row: i32| oy + row as f32 * m.height;

    for (i, b) in mirror.blocks.iter().enumerate() {
        let end = mirror.block_end(i);
        let first = b.prompt_row.max(0);
        let last = end.min(rows - 1);
        if first > last {
            continue;
        }
        let color = match b.state {
            BlockState::Editing => None,
            BlockState::Running => Some(theme.accent),
            BlockState::Done { exit: Some(0) } => Some(theme.palette[2]),
            BlockState::Done { exit: Some(_) } => Some(theme.palette[1]),
            BlockState::Done { exit: None } => Some(mix(0.35)),
        };
        if let Some(c) = color {
            let y0 = row_y(first);
            let y1 = row_y(last + 1);
            r.rect(bar_x, y0, bar_w, y1 - y0, [c[0], c[1], c[2], 0.85]);
        }

        if b.prompt_row > 0 && b.prompt_row < rows {
            r.rect(area.x, row_y(b.prompt_row) - scale.max(1.0) / 2.0, area.w, scale.round().max(1.0), mix(0.10));
        }

        if b.prompt_row >= 0
            && b.prompt_row < rows
            && let Some((text, c)) = block_status(theme, b)
        {
            let w = text.chars().count() as f32 * m.width;
            let x = (right - w).round();
            let y = row_y(b.prompt_row);
            r.rect(x - m.width / 2.0, y, w + m.width / 2.0, m.height, bg);
            label(r, &text, x, y, w + m.width, c, false);
        }
    }

    if let Some(i) = mirror.block_at(0)
        && let b = &mirror.blocks[i]

        && b.output_row <= 0
        && mirror.block_end(i) >= 1
        && b.cmd.is_some()
    {
        let y = oy;
        let h = m.height;
        r.rect(area.x, area.y, area.w, (y - area.y) + h + scale * 2.0, mix(0.07));
        r.rect(area.x, y + h + scale, area.w, scale.round().max(1.0), mix(0.16));
        let status = block_status(theme, b);
        let status_w = status.as_ref().map_or(0.0, |(t, _)| (t.chars().count() + 1) as f32 * m.width);
        let text = format!("❯ {}", b.cmd.as_deref().unwrap_or(""));
        label(r, &text, ox, y, right - ox - status_w, theme.foreground, true);
        if let Some((t, c)) = status {
            let w = t.chars().count() as f32 * m.width;
            label(r, &t, (right - w).round(), y, w + m.width, c, false);
        }
    }

    if let Some(id) = hovered
        && let Some(i) = mirror.blocks.iter().position(|b| b.id == id)
        && let b = &mirror.blocks[i]
        && b.state != BlockState::Editing
    {
        let row = if b.prompt_row < 0 { 0 } else { b.prompt_row };
        if row < rows {
            let status_w = block_status(theme, b).map_or(0.0, |(t, _)| (t.chars().count() + 2) as f32 * m.width);
            let mut x = right - status_w;
            let y = row_y(row);
            let folded = mirror.folds.iter().any(|f| f.block == b.id);
            let fold_label = if folded { "▸ unfold" } else { "▾ fold" };
            let actions = [
                (BlockAction::ToggleFold, fold_label, if folded { "▸" } else { "▾" }),
                (BlockAction::Rerun, "↻ rerun", "↻"),
                (BlockAction::SaveOutput, "↓ save", "↓"),
                (BlockAction::CopyOutput, "⧉ output", "⧉"),
                (BlockAction::CopyCommand, "❯ cmd", "❯"),
            ];
            let shown = || actions.iter().filter(|(a, _, _)| *a != BlockAction::CopyCommand || b.cmd.is_some());

            let full_w: f32 = shown().map(|(_, t, _)| (t.chars().count() as f32 + 1.5) * m.width).sum();
            let compact = full_w > (right - ox) / 3.0;
            for &(action, long, short) in shown() {
                let text = if compact { short } else { long };
                let w = (text.chars().count() as f32 + 1.0) * m.width;
                x -= w + m.width / 2.0;
                let rect = Rect { x, y, w, h: m.height };
                r.rounded_rect(rect.x, rect.y, rect.w, rect.h, theme::RADIUS_SMALL * scale, mix(0.16));
                label(r, text, x + m.width / 2.0, y, w, theme.foreground, false);
                buttons.push(BlockButton { rect, block: b.id, action });
            }
        }
    }
    buttons
}

fn redact_secrets(r: &mut Renderer, theme: &Theme, mirror: &Mirror, ox: f32, oy: f32, scale: f32) {
    let m = r.cell_metrics();
    let (fg, bg) = (theme.foreground, theme.background);
    let cover = [bg[0] + (fg[0] - bg[0]) * 0.18, bg[1] + (fg[1] - bg[1]) * 0.18, bg[2] + (fg[2] - bg[2]) * 0.18, 1.0];
    let dots = [bg[0] + (fg[0] - bg[0]) * 0.4, bg[1] + (fg[1] - bg[1]) * 0.4, bg[2] + (fg[2] - bg[2]) * 0.4, 1.0];
    let mut row = 0;
    while row < mirror.lines.len() {
        let mut text = String::new();
        let mut map: Vec<(usize, usize)> = Vec::new();
        let mut r_end = row;
        loop {
            let line = &mirror.lines[r_end];
            for (col, cell) in line.iter().enumerate() {
                if cell.flags & (flags::WIDE_CHAR_SPACER | flags::LEADING_WIDE_CHAR_SPACER) != 0 {
                    continue;
                }
                let ch = if cell.ch == '\0' { ' ' } else { cell.ch };
                for _ in 0..ch.len_utf8() {
                    map.push((r_end, col));
                }
                text.push(ch);
            }
            let wraps = line.last().is_some_and(|c| c.flags & flags::WRAPLINE != 0);
            if !wraps || r_end + 1 >= mirror.lines.len() {
                break;
            }
            r_end += 1;
        }
        for found in zhell_secrets::find(&text) {
            let cells: Vec<(usize, usize)> = map[found.range.clone()].to_vec();

            let mut i = 0;
            while i < cells.len() {
                let (rr, c0) = cells[i];
                let mut c1 = c0;
                while i < cells.len() && cells[i].0 == rr {
                    c1 = cells[i].1;
                    i += 1;
                }
                let x = ox + c0 as f32 * m.width;
                let y = oy + rr as f32 * m.height;
                let w = (c1 - c0 + 1) as f32 * m.width;

                let o = scale.round().max(1.0);
                r.rounded_rect(x - o, y - o, w + 2.0 * o, m.height + 2.0 * o, 2.0 * scale, cover);
                let mut dx = x + m.width / 2.0;
                while dx < x + w - 2.0 * scale {
                    let d = (3.0 * scale).round();
                    r.rounded_rect(dx - d / 2.0, y + (m.height - d) / 2.0, d, d, d / 2.0, dots);
                    dx += m.width;
                }
            }
        }
        row = r_end + 1;
    }
}

pub fn mode_badge(r: &mut Renderer, theme: &Theme, area: Rect, name: &str, hint: &str, scale: f32) {
    let m = r.cell_metrics();
    let pad = (6.0 * scale).round();
    let chip_w = name.chars().count() as f32 * m.width + pad * 2.0;
    let hint_w = hint.chars().count() as f32 * m.width;
    let h = m.height + (4.0 * scale).round();
    let x = (area.x + area.w - chip_w - (8.0 * scale)).round();
    let y = (area.y + 6.0 * scale).round();
    let (fg, bg) = (theme.foreground, theme.background);

    if area.w > chip_w + hint_w + 60.0 * scale {
        let hx = x - hint_w - pad * 3.0;
        let plate = [bg[0] + (fg[0] - bg[0]) * 0.06, bg[1] + (fg[1] - bg[1]) * 0.06, bg[2] + (fg[2] - bg[2]) * 0.06, 1.0];
        let edge = [bg[0] + (fg[0] - bg[0]) * 0.16, bg[1] + (fg[1] - bg[1]) * 0.16, bg[2] + (fg[2] - bg[2]) * 0.16, 1.0];
        r.rounded_rect(hx, y, hint_w + pad * 2.0, h, theme::RADIUS_SMALL * scale, plate);
        r.rounded_outline([hx, y, hint_w + pad * 2.0, h], theme::RADIUS_SMALL * scale, scale.round().max(1.0), edge);
        label(r, hint, hx + pad, y + (h - m.height) / 2.0, hint_w + m.width, [fg[0], fg[1], fg[2], 0.7], false);
    }
    r.rounded_rect(x, y, chip_w, h, theme::RADIUS_SMALL * scale, theme.accent);
    label(r, name, x + pad, y + (h - m.height) / 2.0, chip_w, [1.0, 1.0, 1.0, 1.0], true);
}

pub fn toast(r: &mut Renderer, theme: &Theme, text: &str, error: bool, win_w: f32, win_h: f32, scale: f32) {
    let m = r.cell_metrics();
    let (fg, bg) = (theme.foreground, theme.background);
    let pad = (12.0 * scale).round();
    let max_chars = (((win_w * 0.7) - pad * 2.0) / m.width).max(10.0) as usize;

    let mut lines: Vec<String> = Vec::new();
    for word in text.split(' ') {
        match lines.last_mut() {
            Some(l) if l.chars().count() + 1 + word.chars().count() <= max_chars => {
                l.push(' ');
                l.push_str(word);
            }
            _ => lines.push(word.to_owned()),
        }
    }
    if lines.len() > 3 {
        lines.truncate(3);
        lines[2].push('…');
    }
    let width_chars = lines.iter().map(|l| l.chars().count()).max().unwrap_or(0).min(max_chars);
    let w = width_chars as f32 * m.width + pad * 2.0;
    let h = lines.len() as f32 * m.height + pad * 1.4;
    let x = (win_w - w - pad * 1.5).round();
    let y = (win_h - h - pad * 1.5).round();
    let fill = [bg[0] + (fg[0] - bg[0]) * 0.1, bg[1] + (fg[1] - bg[1]) * 0.1, bg[2] + (fg[2] - bg[2]) * 0.1, 0.97];
    r.rounded_rect(x, y, w, h, theme::RADIUS * scale, fill);
    r.rounded_rect(x, y, (3.0 * scale).round(), h, 1.5 * scale, if error { theme.palette[1] } else { theme.accent });
    for (i, l) in lines.iter().enumerate() {
        label(r, l, x + pad, (y + pad * 0.7 + i as f32 * m.height).round(), w - pad * 1.5, fg, false);
    }
}
