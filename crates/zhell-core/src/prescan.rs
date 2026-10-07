use base64::Engine as _;

pub const ZHELL_OSC: &str = "6973";

const MAX_PAYLOAD: usize = 8192;

const MAX_IMAGE_PAYLOAD: usize = 64 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OscEvent {
    PromptStart,
    CommandStart,
    OutputStart,
    CommandFinished { exit_code: Option<i32> },
    Cwd(String),

    RemoteCwd { host: String, path: String },
    CommandText(String),

    Ready,

    KittyGraphics(Vec<u8>),

    Sixel(Vec<u8>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Osc,
    Apc,
    Dcs,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Ground,
    Esc,
    Osc,

    OscEsc,
}

#[derive(Debug)]
pub struct OscPrescanner {
    state: State,
    kind: Kind,
    payload: Vec<u8>,
    overflow: bool,
}

impl Default for OscPrescanner {
    fn default() -> Self {
        Self::new()
    }
}

impl OscPrescanner {
    pub fn new() -> Self {
        Self { state: State::Ground, kind: Kind::Osc, payload: Vec::new(), overflow: false }
    }

    pub fn scan(&mut self, bytes: &[u8], out: &mut Vec<(usize, OscEvent)>) {
        for (i, &b) in bytes.iter().enumerate() {
            match self.state {
                State::Ground => {
                    if b == 0x1b {
                        self.state = State::Esc;
                    }
                }
                State::Esc => {
                    let kind = match b {
                        b']' => Some(Kind::Osc),
                        b'_' => Some(Kind::Apc),
                        b'P' => Some(Kind::Dcs),
                        _ => None,
                    };
                    self.state = match kind {
                        Some(k) => {
                            self.kind = k;
                            self.payload.clear();
                            self.overflow = false;
                            State::Osc
                        }
                        None if b == 0x1b => State::Esc,
                        None => State::Ground,
                    };
                }
                State::Osc => match b {
                    0x07 if self.kind == Kind::Osc => self.finish(i + 1, out),
                    0x1b => self.state = State::OscEsc,

                    0x18 | 0x1a => self.state = State::Ground,
                    _ => self.push(b),
                },
                State::OscEsc => {
                    if b == b'\\' {
                        self.finish(i + 1, out);
                    } else {
                        self.state = if b == b']' {
                            self.kind = Kind::Osc;
                            self.payload.clear();
                            self.overflow = false;
                            State::Osc
                        } else {
                            State::Ground
                        };
                    }
                }
            }
        }
    }

    fn push(&mut self, b: u8) {
        let max = if self.kind == Kind::Osc { MAX_PAYLOAD } else { MAX_IMAGE_PAYLOAD };

        let interesting = match self.kind {
            Kind::Osc => true,
            Kind::Apc => self.payload.first().is_none_or(|&c| c == b'G'),
            Kind::Dcs => true,
        };
        if interesting && self.payload.len() < max {
            self.payload.push(b);
        } else {
            self.overflow = true;
        }
    }

    fn finish(&mut self, end: usize, out: &mut Vec<(usize, OscEvent)>) {
        self.state = State::Ground;
        let ev = if self.overflow {
            None
        } else {
            match self.kind {
                Kind::Osc => parse_osc(&self.payload),
                Kind::Apc => self.payload.strip_prefix(b"G").map(|p| OscEvent::KittyGraphics(p.to_vec())),
                Kind::Dcs => {
                    let q = self.payload.iter().position(|&c| !(c.is_ascii_digit() || c == b';'));
                    (q.is_some_and(|q| self.payload[q] == b'q')).then(|| OscEvent::Sixel(std::mem::take(&mut self.payload)))
                }
            }
        };
        if let Some(ev) = ev {
            out.push((end, ev));
        }
        self.payload.clear();
    }
}

fn parse_osc(payload: &[u8]) -> Option<OscEvent> {
    let s = std::str::from_utf8(payload).ok()?;
    let (num, rest) = s.split_once(';')?;
    let mut parts = rest.split(';');
    match num {
        "133" => match parts.next()? {
            "A" => Some(OscEvent::PromptStart),
            "B" => Some(OscEvent::CommandStart),
            "C" => Some(OscEvent::OutputStart),
            "D" => {
                let exit_code = parts.next().and_then(|c| c.trim().parse().ok());
                Some(OscEvent::CommandFinished { exit_code })
            }
            _ => None,
        },
        "7" => {
            let (host, path) = parse_file_url(rest)?;
            Some(if is_local_host(&host) { OscEvent::Cwd(path) } else { OscEvent::RemoteCwd { host, path } })
        }

        "633" => match parts.next()? {
            "A" => Some(OscEvent::PromptStart),
            "B" => Some(OscEvent::CommandStart),
            "C" => Some(OscEvent::OutputStart),
            "D" => {
                let exit_code = parts.next().and_then(|c| c.trim().parse().ok());
                Some(OscEvent::CommandFinished { exit_code })
            }
            "E" => {
                let cmd = rest.split_once(';').map_or("", |(_, r)| r);
                let cmd = cmd.split(';').next().unwrap_or("");
                unescape_633(cmd).map(OscEvent::CommandText)
            }
            "P" => {
                let prop = rest.split_once(';')?.1;
                prop.strip_prefix("Cwd=").and_then(unescape_633).map(OscEvent::Cwd)
            }
            _ => None,
        },
        ZHELL_OSC => {
            if rest == "ready" {
                return Some(OscEvent::Ready);
            }
            let b64 = rest.strip_prefix("cmd=")?;
            let bytes = base64::engine::general_purpose::STANDARD.decode(b64).ok()?;
            String::from_utf8(bytes).ok().map(OscEvent::CommandText)
        }
        _ => None,
    }
}

fn unescape_633(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' && i + 1 < b.len() {
            if b[i + 1] == b'\\' {
                out.push(b'\\');
                i += 2;
                continue;
            }
            if b[i + 1] == b'x' && i + 3 < b.len() {
                let hex = std::str::from_utf8(&b[i + 2..i + 4]).ok()?;
                out.push(u8::from_str_radix(hex, 16).ok()?);
                i += 4;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8(out).ok()
}

fn parse_file_url(url: &str) -> Option<(String, String)> {
    let rest = url.strip_prefix("file://")?;
    let slash = rest.find('/')?;
    let host = rest[..slash].to_owned();
    let decoded = percent_decode(&rest[slash..])?;

    let b = decoded.as_bytes();
    if b.len() >= 3 && b[0] == b'/' && b[1].is_ascii_alphabetic() && b[2] == b':' {
        return Some((host, decoded[1..].to_owned()));
    }
    Some((host, decoded))
}

pub fn local_hostname() -> &'static str {
    static NAME: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    NAME.get_or_init(|| {
        let from_file = std::fs::read_to_string("/etc/hostname").ok().map(|h| h.trim().to_owned()).filter(|h| !h.is_empty());
        from_file
            .or_else(|| std::env::var("HOSTNAME").ok().filter(|h| !h.is_empty()))
            .or_else(|| std::env::var("COMPUTERNAME").ok().filter(|h| !h.is_empty()))
            .unwrap_or_else(|| "localhost".into())
    })
}

fn is_local_host(host: &str) -> bool {
    let short = |h: &str| h.split('.').next().unwrap_or(h).to_ascii_lowercase();
    host.is_empty() || host.eq_ignore_ascii_case("localhost") || short(host) == short(local_hostname())
}

fn percent_decode(s: &str) -> Option<String> {
    let mut out = Vec::with_capacity(s.len());
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let hex = std::str::from_utf8(&b[i + 1..i + 3]).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scan_all(chunks: &[&[u8]]) -> Vec<(usize, usize, OscEvent)> {
        let mut p = OscPrescanner::new();
        let mut res = Vec::new();
        for (ci, c) in chunks.iter().enumerate() {
            let mut out = Vec::new();
            p.scan(c, &mut out);
            res.extend(out.into_iter().map(|(o, e)| (ci, o, e)));
        }
        res
    }

    #[test]
    fn osc133_full_cycle() {
        let s = b"\x1b]133;A\x07$ \x1b]133;B\x07ls\r\n\x1b]133;C\x07out\r\n\x1b]133;D;2\x1b\\";
        let ev: Vec<_> = scan_all(&[s]).into_iter().map(|(_, _, e)| e).collect();
        assert_eq!(
            ev,
            vec![
                OscEvent::PromptStart,
                OscEvent::CommandStart,
                OscEvent::OutputStart,
                OscEvent::CommandFinished { exit_code: Some(2) },
            ]
        );
    }

    #[test]
    fn offsets_point_past_terminator() {
        let s = b"ab\x1b]133;A\x07cd";
        let r = scan_all(&[s]);
        assert_eq!(r[0].1, 10);
        assert_eq!(&s[r[0].1..], b"cd");
    }

    #[test]
    fn split_across_chunks() {
        let r = scan_all(&[b"x\x1b]13", b"3;D;0\x1b", b"\\y"]);
        assert_eq!(r, vec![(2, 1, OscEvent::CommandFinished { exit_code: Some(0) })]);
    }

    #[test]
    fn cwd_and_command_text() {
        let cmd = base64::engine::general_purpose::STANDARD.encode("echo ä; ls");
        let s = format!(
            "\x1b]7;file://host/home/me/my%20dir\x07\x1b]6973;cmd={cmd}\x07\x1b]7;file:///C:/Users/z\x07"
        );
        let ev: Vec<_> = scan_all(&[s.as_bytes()]).into_iter().map(|(_, _, e)| e).collect();
        assert_eq!(
            ev,
            vec![
                OscEvent::RemoteCwd { host: "host".into(), path: "/home/me/my dir".into() },
                OscEvent::CommandText("echo ä; ls".into()),
                OscEvent::Cwd("C:/Users/z".into()),
            ]
        );
    }

    #[test]
    fn ignores_other_and_oversized_osc() {
        let big = format!("\x1b]52;c;{}\x07\x1b]0;title\x07", "A".repeat(MAX_PAYLOAD + 10));
        assert!(scan_all(&[big.as_bytes()]).is_empty());
    }

    #[test]
    fn vscode_633_sequences() {
        let s = "\x1b]633;A\x07\x1b]633;E;echo a\\x3b b \\\\x;nonce123\x07\x1b]633;C\x07\x1b]633;D;1\x07\x1b]633;P;Cwd=/tmp/a b\x07";
        let ev: Vec<_> = scan_all(&[s.as_bytes()]).into_iter().map(|(_, _, e)| e).collect();
        assert_eq!(
            ev,
            vec![
                OscEvent::PromptStart,
                OscEvent::CommandText("echo a; b \\x".into()),
                OscEvent::OutputStart,
                OscEvent::CommandFinished { exit_code: Some(1) },
                OscEvent::Cwd("/tmp/a b".into()),
            ]
        );
    }

    #[test]
    fn image_sequences() {
        let s = b"\x1b_Ga=T,f=100;AAAA\x1b\\x\x1bP0;1q#0;2;0;0;0~-\x1b\\\x1b_Xnot-graphics\x1b\\\x1bP+q544e\x1b\\";
        let ev: Vec<_> = scan_all(&[s]).into_iter().map(|(_, _, e)| e).collect();
        assert_eq!(
            ev,
            vec![OscEvent::KittyGraphics(b"a=T,f=100;AAAA".to_vec()), OscEvent::Sixel(b"0;1q#0;2;0;0;0~-".to_vec())]
        );
    }

    #[test]
    fn missing_exit_code() {
        let ev = scan_all(&[b"\x1b]133;D\x07"]);
        assert_eq!(ev[0].2, OscEvent::CommandFinished { exit_code: None });
    }
}
