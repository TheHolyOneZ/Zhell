use std::sync::LazyLock;

use regex::Regex;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkKind {
    Url,

    Path { line: Option<u32>, col: Option<u32> },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkMatch {
    pub start: usize,
    pub end: usize,

    pub target: String,
    pub kind: LinkKind,
}

const SCHEMES: &str = r"https?://|ftp://|file://|mailto:";

static URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(r#"(?:{SCHEMES})[^\s<>"'`{{}}|\\^]+"#)).expect("url regex")
});

static PATH: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?x)
        (?P<path>
            (?: ~ | \.{1,2} | [A-Za-z]: )? [/\\]? [\w.@+\-]+ (?: [/\\] [\w.@+\-]+ )*
        )
        (?: : (?P<line>\d+) (?: : (?P<col>\d+) )?
          | \( (?P<pline>\d+) (?: , (?P<pcol>\d+) )? \) )?
        ",
    )
    .expect("path regex")
});

fn trim_trailing(s: &str) -> &str {
    let mut s = s.trim_end_matches(['.', ',', ';', ':', '!', '?', '\'', '"']);
    for (open, close) in [('(', ')'), ('[', ']')] {
        while s.ends_with(close) && s.matches(close).count() > s.matches(open).count() {
            s = &s[..s.len() - 1];
        }
    }
    s
}

fn char_index(text: &str, byte: usize) -> usize {
    text[..byte].chars().count()
}

pub fn find(text: &str) -> Vec<LinkMatch> {
    let mut out: Vec<LinkMatch> = Vec::new();
    let mut taken: Vec<(usize, usize)> = Vec::new();

    for m in URL.find_iter(text) {
        let url = trim_trailing(m.as_str());
        let (s, e) = (m.start(), m.start() + url.len());
        taken.push((s, e));
        out.push(LinkMatch {
            start: char_index(text, s),
            end: char_index(text, e),
            target: url.to_owned(),
            kind: LinkKind::Url,
        });
    }

    for c in PATH.captures_iter(text) {
        let whole = c.get(0).expect("match");
        let path_m = c.name("path").expect("path group");
        let (s, mut e) = (whole.start(), whole.end());
        if taken.iter().any(|&(ts, te)| s < te && e > ts) {
            continue;
        }

        let boundary = text[..s].chars().next_back().is_none_or(|c| c.is_whitespace() || "([{<\"'`=,".contains(c));
        if !boundary {
            continue;
        }
        let path = trim_trailing(path_m.as_str());
        let num = |name: &str| c.name(name).and_then(|m| m.as_str().parse::<u32>().ok());
        let line = num("line").or_else(|| num("pline"));
        let col = num("col").or_else(|| num("pcol"));
        let has_sep = path.contains('/') || path.contains('\\');

        if path.is_empty() || (!has_sep && line.is_none()) || !path.chars().any(|c| c.is_alphanumeric()) {
            continue;
        }

        if !has_sep && !path.contains('.') {
            continue;
        }
        if line.is_none() {
            e = path_m.start() + path.len();
        }
        out.push(LinkMatch {
            start: char_index(text, s),
            end: char_index(text, e),
            target: path.to_owned(),
            kind: LinkKind::Path { line, col },
        });
    }
    out.sort_by_key(|m| m.start);
    out
}

pub fn at(text: &str, pos: usize) -> Option<LinkMatch> {
    find(text).into_iter().find(|m| pos >= m.start && pos < m.end)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn targets(t: &str) -> Vec<(String, LinkKind)> {
        find(t).into_iter().map(|m| (m.target, m.kind)).collect()
    }

    #[test]
    fn urls() {
        assert_eq!(
            targets("see https://zsync.eu/zhell. and (http://a.b/c_(d)) ok"),
            vec![
                ("https://zsync.eu/zhell".into(), LinkKind::Url),
                ("http://a.b/c_(d)".into(), LinkKind::Url),
            ]
        );
        assert!(targets("javascript:alert(1) foo://bar").is_empty());
    }

    #[test]
    fn rust_and_compiler_paths() {
        let p = |line, col| LinkKind::Path { line, col };
        assert_eq!(
            targets("  --> src/manifest.rs:88:5"),
            vec![("src/manifest.rs".into(), p(Some(88), Some(5)))]
        );
        assert_eq!(targets("main.rs:42: error"), vec![("main.rs".into(), p(Some(42), None))]);
        assert_eq!(targets("C:\\src\\a.cpp(12,3): warning"), vec![("C:\\src\\a.cpp".into(), p(Some(12), Some(3)))]);
        assert_eq!(targets("edit ~/.config/zhell/zhell.toml"), vec![("~/.config/zhell/zhell.toml".into(), p(None, None))]);
        assert_eq!(targets("./run.sh, then"), vec![("./run.sh".into(), p(None, None))]);
    }

    #[test]
    fn plain_words_and_times_are_not_links() {
        assert!(targets("hello world at 12:30 version 1.2").is_empty());
        assert!(targets("ratio 3/4").len() == 1, "a/b with a slash is a candidate; existence is checked later");
    }

    #[test]
    fn char_positions_and_lookup() {
        let t = "ä https://x.io/ü end";
        let m = &find(t)[0];
        assert_eq!((m.start, m.end), (2, 16));
        assert_eq!(at(t, 5).unwrap().target, "https://x.io/ü");
        assert!(at(t, 0).is_none());
    }
}
