use std::fmt::Write;

use zhell_proto::{Cell, Color, flags};

use crate::theme::{Rgba, Theme};

fn hex(c: Rgba) -> String {
    let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", b(c[0]), b(c[1]), b(c[2]))
}

fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            c => out.push(c),
        }
    }
    out
}

#[derive(Clone, PartialEq)]
struct Style {
    fg: String,
    bg: Option<String>,
    bold: bool,
    italic: bool,
    underline: bool,
    strike: bool,
    dim: bool,
    link: Option<String>,
}

fn style_of(c: &Cell, theme: &Theme) -> Style {
    let inverse = c.flags & flags::INVERSE != 0;
    let (mut fg, mut bg) = (c.fg, c.bg);
    if inverse {
        std::mem::swap(&mut fg, &mut bg);
    }
    let default_bg = !inverse && matches!(bg, Color::Named(n) if n == zhell_proto::named::BACKGROUND);
    let mut fg = theme.resolve(fg);
    if c.flags & flags::HIDDEN != 0 {
        fg = theme.resolve(bg);
    }
    Style {
        fg: hex(fg),
        bg: (!default_bg).then(|| hex(theme.resolve(bg))),
        bold: c.flags & flags::BOLD != 0,
        italic: c.flags & flags::ITALIC != 0,
        underline: c.flags & (flags::UNDERLINE | flags::DOUBLE_UNDERLINE | flags::UNDERCURL | flags::DOTTED_UNDERLINE | flags::DASHED_UNDERLINE) != 0,
        strike: c.flags & flags::STRIKEOUT != 0,
        dim: c.flags & flags::DIM != 0,
        link: c.hyperlink.clone().filter(|l| l.starts_with("http://") || l.starts_with("https://")),
    }
}

fn open_span(out: &mut String, s: &Style, fg_default: &str) {
    let mut css = String::new();
    if s.fg != fg_default {
        let _ = write!(css, "color:{};", s.fg);
    }
    if let Some(bg) = &s.bg {
        let _ = write!(css, "background:{bg};");
    }
    if s.bold {
        css.push_str("font-weight:700;");
    }
    if s.italic {
        css.push_str("font-style:italic;");
    }
    match (s.underline, s.strike) {
        (true, true) => css.push_str("text-decoration:underline line-through;"),
        (true, false) => css.push_str("text-decoration:underline;"),
        (false, true) => css.push_str("text-decoration:line-through;"),
        _ => {}
    }
    if s.dim {
        css.push_str("opacity:.6;");
    }
    if let Some(link) = &s.link {
        let _ = write!(out, "<a href=\"{}\" style=\"{css}\">", escape(link));
    } else if !css.is_empty() {
        let _ = write!(out, "<span style=\"{css}\">");
    } else {
        out.push_str("<span>");
    }
}

fn close_span(out: &mut String, s: &Style) {
    out.push_str(if s.link.is_some() { "</a>" } else { "</span>" });
}

fn body(rows: &[Vec<Cell>], theme: &Theme) -> String {
    let fg_default = hex(theme.foreground);
    let mut out = String::new();
    for row in rows {
        let end = row
            .iter()
            .rposition(|c| c.ch != ' ' || !matches!(c.bg, Color::Named(n) if n == zhell_proto::named::BACKGROUND) || c.flags & flags::INVERSE != 0)
            .map_or(0, |i| i + 1);
        let mut current: Option<Style> = None;
        let mut text = String::new();
        for c in &row[..end] {
            if c.flags & (flags::WIDE_CHAR_SPACER | flags::LEADING_WIDE_CHAR_SPACER) != 0 {
                continue;
            }
            let s = style_of(c, theme);
            if current.as_ref() != Some(&s) {
                if let Some(prev) = current.take() {
                    out.push_str(&escape(&text));
                    text.clear();
                    close_span(&mut out, &prev);
                }
                open_span(&mut out, &s, &fg_default);
                current = Some(s);
            }
            text.push(if c.ch == '\t' { ' ' } else { c.ch });
            text.extend(&c.zerowidth);
        }
        if let Some(prev) = current {
            out.push_str(&escape(&text));
            close_span(&mut out, &prev);
        }
        let wrapped = row.last().is_some_and(|c| c.flags & flags::WRAPLINE != 0);
        if !wrapped {
            out.push('\n');
        }
    }
    out.trim_end_matches('\n').to_owned()
}

pub fn html(rows: &[Vec<Cell>], theme: &Theme, title: &str, font_family: &str) -> String {
    let bg = hex(theme.background);
    let fg = hex(theme.foreground);
    let border = hex([
        theme.background[0] + (theme.foreground[0] - theme.background[0]) * 0.14,
        theme.background[1] + (theme.foreground[1] - theme.background[1]) * 0.14,
        theme.background[2] + (theme.foreground[2] - theme.background[2]) * 0.14,
        1.0,
    ]);
    let muted = hex([
        theme.background[0] + (theme.foreground[0] - theme.background[0]) * 0.45,
        theme.background[1] + (theme.foreground[1] - theme.background[1]) * 0.45,
        theme.background[2] + (theme.foreground[2] - theme.background[2]) * 0.45,
        1.0,
    ]);
    let accent = hex(theme.accent);
    let font = if font_family.trim().is_empty() { String::new() } else { format!("\"{}\", ", escape(font_family.trim())) };
    format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="generator" content="Zhell">
<title>{title}</title>
<style>
  :root {{ color-scheme: dark light; }}
  body {{ margin: 0; padding: 32px 16px; background: color-mix(in srgb, {bg} 88%, #000); font: 14px/1.45 {font}"JetBrains Mono", "Cascadia Code", "Fira Code", ui-monospace, Menlo, Consolas, monospace; }}
  .term {{ max-width: 1100px; margin: 0 auto; background: {bg}; color: {fg}; border: 1px solid {border}; border-radius: 5px; overflow: hidden; box-shadow: 0 1px 2px #0003, 0 6px 12px #0002, 0 24px 48px #0002; }}
  .bar {{ display: flex; align-items: center; gap: 8px; padding: 8px 14px; border-bottom: 1px solid {border}; color: {muted}; font-size: 12px; }}
  .bar span {{ white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }}
  .bar b {{ flex: none; display: inline-grid; place-items: center; width: 16px; height: 16px; border-radius: 4px; background: {accent}; color: #fff; font-size: 11px; }}
  pre {{ margin: 0; padding: 14px 16px 16px; overflow-x: auto; font: inherit; white-space: pre; }}
  a {{ color: inherit; }}
</style>
</head>
<body>
<div class="term">
<div class="bar"><b>Z</b><span>{title}</span></div>
<pre>{body}</pre>
</div>
</body>
</html>
"#,
        title = escape(title),
        body = body(rows, theme),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell(ch: char, fg: Color, f: u16) -> Cell {
        Cell { ch, fg, flags: f, ..Default::default() }
    }

    #[test]
    fn styled_runs_and_escaping() {
        let theme = Theme::from_spec(&zhell_core::config::Config::default().load_theme(None).unwrap());
        let red = Color::Named(1);
        let fgc = Color::Named(zhell_proto::named::FOREGROUND);
        let rows = vec![
            vec![cell('<', fgc, 0), cell('a', red, flags::BOLD), cell('b', red, flags::BOLD), cell(' ', fgc, 0), cell(' ', fgc, 0)],
            vec![cell('x', fgc, flags::WRAPLINE)],
            vec![cell('y', fgc, 0)],
        ];
        let b = body(&rows, &theme);
        let red_hex = hex(theme.palette[1]);
        assert_eq!(b, format!("<span>&lt;</span><span style=\"color:{red_hex};font-weight:700;\">ab</span>\n<span>x</span><span>y</span>"));
        let page = html(&rows, &theme, "ls <dir>", "");
        assert!(page.contains("<title>ls &lt;dir&gt;</title>") && page.starts_with("<!doctype html>"));
    }
}
