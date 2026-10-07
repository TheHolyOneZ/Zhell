use std::fmt::Write as _;
use std::path::PathBuf;

use zhell_daemon::headless::Headless;
use zhell_proto::{Cell, Color, FrameDiff, flags, named};

struct Case {
    name: &'static str,
    cols: u16,
    rows: u16,
    input: &'static str,
}

const CASES: &[Case] = &[
    Case { name: "plain_wrap", cols: 10, rows: 4, input: "hello world, this wraps\r\nnext" },
    Case {
        name: "cursor_movement",
        cols: 12,
        rows: 5,
        input: "\x1b[3;5HX\x1b[1;1HA\x1b[2CB\x1b[2BC\x1b[1DD\x1b[AE\x1b[10GF\x1b[5dG",
    },
    Case {
        name: "erase_in_display_and_line",
        cols: 8,
        rows: 4,
        input: "AAAAAAAA\r\nBBBBBBBB\r\nCCCCCCCC\r\nDDDDDDDD\x1b[2;4H\x1b[K\x1b[3;3H\x1b[1K\x1b[4;5H\x1b[J\x1b[1;7H\x1b[1J",
    },
    Case {
        name: "scroll_region",
        cols: 6,
        rows: 6,
        input: "1\r\n2\r\n3\r\n4\r\n5\r\n6\x1b[2;4r\x1b[4;1H\nnew\x1b[2;1H\x1bMtop\x1b[r",
    },
    Case {
        name: "insert_delete_lines_chars",
        cols: 8,
        rows: 4,
        input: "abcdefgh\r\n12345678\r\nABCDEFGH\x1b[1;3H\x1b[2@\x1b[2;2H\x1b[3P\x1b[2;1H\x1b[L\x1b[4;1H\x1b[M",
    },
    Case {
        name: "tabs",
        cols: 30,
        rows: 3,
        input: "a\tb\tc\r\n\x1b[3g\x1b[5G\x1bH\rx\ty\r\n\x1b[1;20H\x1b[Zz",
    },
    Case {
        name: "sgr_attributes",
        cols: 20,
        rows: 3,
        input: "\x1b[1mB\x1b[22;3mI\x1b[23;4mU\x1b[24;9mS\x1b[0;7mR\x1b[0;2mD\x1b[0;8mH\x1b[0m.\
                \r\n\x1b[31mr\x1b[42mg\x1b[38;5;208mo\x1b[48;2;1;2;3mt\x1b[39;49mn\
                \r\n\x1b[4:3mc\x1b[58;2;255;0;0mu\x1b[4:2md\x1b[0m",
    },
    Case {
        name: "wide_and_combining",
        cols: 6,
        rows: 4,
        input: "a中b\r\ne\u{301}x\r\nabcde中",
    },
    Case {
        name: "alt_screen_restores_main",
        cols: 10,
        rows: 3,
        input: "main\x1b[?1049h\x1b[2J\x1b[Halt screen\x1b[?1049l",
    },
    Case {
        name: "save_restore_cursor_and_origin",
        cols: 10,
        rows: 5,
        input: "\x1b[2;3H\x1b7\x1b[5;9Hx\x1b8y\x1b[2;4r\x1b[?6h\x1b[1;1Ho\x1b[?6l\x1b[r",
    },
    Case { name: "decaln", cols: 5, rows: 3, input: "\x1b#8" },
    Case {
        name: "dec_line_drawing_charset",
        cols: 10,
        rows: 2,
        input: "\x1b(0lqqk\r\nmqqj\x1b(Bok",
    },
    Case {
        name: "hyperlink",
        cols: 12,
        rows: 2,
        input: "\x1b]8;;https://zsync.eu\x1b\\link\x1b]8;;\x1b\\ text",
    },
    Case {
        name: "autowrap_off",
        cols: 5,
        rows: 3,
        input: "\x1b[?7labcdefgh\x1b[?7h\r\n12345678",
    },
    Case {
        name: "shell_marks_are_invisible",
        cols: 12,
        rows: 3,
        input: "\x1b]133;A\x07$ \x1b]133;B\x07ls\r\n\x1b]133;C\x07out\r\n\x1b]133;D;0\x07\x1b]7;file:///tmp\x07",
    },
];

fn color(c: Color) -> String {
    match c {
        Color::Named(named::FOREGROUND) => "fg".into(),
        Color::Named(named::BACKGROUND) => "bg".into(),
        Color::Named(n) => format!("n{n}"),
        Color::Indexed(i) => format!("i{i}"),
        Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
    }
}

fn flag_names(f: u16) -> String {
    const NAMES: &[(u16, &str)] = &[
        (flags::BOLD, "bold"),
        (flags::ITALIC, "italic"),
        (flags::UNDERLINE, "ul"),
        (flags::DOUBLE_UNDERLINE, "ul2"),
        (flags::UNDERCURL, "curl"),
        (flags::DOTTED_UNDERLINE, "dotted"),
        (flags::DASHED_UNDERLINE, "dashed"),
        (flags::INVERSE, "inverse"),
        (flags::DIM, "dim"),
        (flags::HIDDEN, "hidden"),
        (flags::STRIKEOUT, "strike"),
        (flags::WIDE_CHAR, "wide"),
        (flags::WIDE_CHAR_SPACER, "spacer"),
        (flags::LEADING_WIDE_CHAR_SPACER, "lspacer"),
        (flags::WRAPLINE, "wrap"),
    ];
    NAMES.iter().filter(|(b, _)| f & b != 0).map(|(_, n)| *n).collect::<Vec<_>>().join(",")
}

fn style(c: &Cell) -> String {
    let mut parts = Vec::new();
    if c.fg != Color::Named(named::FOREGROUND) {
        parts.push(format!("fg={}", color(c.fg)));
    }
    if c.bg != Color::Named(named::BACKGROUND) {
        parts.push(format!("bg={}", color(c.bg)));
    }
    if let Some(u) = c.underline_color {
        parts.push(format!("ulc={}", color(u)));
    }
    let f = flag_names(c.flags);
    if !f.is_empty() {
        parts.push(f);
    }
    if let Some(h) = &c.hyperlink {
        parts.push(format!("link={h}"));
    }
    parts.join(" ")
}

fn snapshot(f: &FrameDiff) -> String {
    let mut out = String::new();
    let c = f.cursor;
    writeln!(out, "size {}x{}  cursor row={} col={} {:?}", f.cols, f.rows, c.row, c.col, c.shape).unwrap();
    let border = "-".repeat(f.cols as usize);
    writeln!(out, "+{border}+").unwrap();
    for l in &f.lines {
        let text: String = l
            .cells
            .iter()
            .map(|c| {
                if c.flags & flags::WIDE_CHAR_SPACER != 0 {
                    String::new()
                } else {
                    let mut s = String::from(if c.ch == '\t' { '→' } else { c.ch });
                    s.extend(&c.zerowidth);
                    s
                }
            })
            .collect();
        writeln!(out, "|{text}|").unwrap();
    }
    writeln!(out, "+{border}+").unwrap();

    for l in &f.lines {
        let mut start = 0;
        while start < l.cells.len() {
            let s = style(&l.cells[start]);
            let mut end = start + 1;
            while end < l.cells.len() && style(&l.cells[end]) == s {
                end += 1;
            }
            if !s.is_empty() {
                writeln!(out, "row {} col {}..{}: {s}", l.row, start, end).unwrap();
            }
            start = end;
        }
    }
    out
}

fn snap_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/vt")
}

#[test]
fn vt_golden() {
    let update = std::env::var_os("UPDATE_SNAPSHOTS").is_some();
    let mut failures = Vec::new();
    for case in CASES {
        let mut t = Headless::new(case.cols, case.rows);
        t.feed(case.input.as_bytes());
        let actual = snapshot(&t.frame());
        let path = snap_dir().join(format!("{}.snap", case.name));
        if update {
            std::fs::create_dir_all(snap_dir()).unwrap();
            std::fs::write(&path, &actual).unwrap();
            continue;
        }
        match std::fs::read_to_string(&path) {
            Ok(expected) if expected.replace("\r\n", "\n") == actual => {}
            Ok(expected) => failures.push(format!(
                "{}: snapshot differs\n--- expected\n{expected}--- actual\n{actual}",
                case.name
            )),
            Err(_) => failures.push(format!("{}: missing snapshot (run with UPDATE_SNAPSHOTS=1)", case.name)),
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

#[test]
fn split_feeding_matches_whole_feeding() {
    for case in CASES {
        let mut whole = Headless::new(case.cols, case.rows);
        whole.feed(case.input.as_bytes());
        let mut split = Headless::new(case.cols, case.rows);
        for b in case.input.as_bytes() {
            split.feed(std::slice::from_ref(b));
        }
        assert_eq!(snapshot(&whole.frame()), snapshot(&split.frame()), "case {}", case.name);
    }
}
