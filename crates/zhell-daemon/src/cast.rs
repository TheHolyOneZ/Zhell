use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

pub struct Cast {
    file: BufWriter<File>,
    start: Instant,

    partial: Vec<u8>,
    pub path: PathBuf,
}

impl Cast {
    pub fn create(path: &Path, cols: u16, rows: u16, title: Option<&str>) -> std::io::Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut file = BufWriter::new(File::create(path)?);
        let timestamp = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let mut header = serde_json::json!({
            "version": 2,
            "width": cols,
            "height": rows,
            "timestamp": timestamp,
            "env": { "TERM": "xterm-256color", "SHELL": std::env::var("SHELL").unwrap_or_default() },
        });
        if let Some(t) = title {
            header["title"] = t.into();
        }
        writeln!(file, "{header}")?;
        file.flush()?;
        Ok(Self { file, start: Instant::now(), partial: Vec::new(), path: path.to_owned() })
    }

    fn event(&mut self, kind: &str, data: &str) {
        let t = self.start.elapsed().as_secs_f64();
        let line = serde_json::to_string(&(t, kind, data)).unwrap_or_default();

        let _ = writeln!(self.file, "{line}").and_then(|()| self.file.flush());
    }

    pub fn output(&mut self, bytes: &[u8]) {
        self.partial.extend_from_slice(bytes);
        let valid = match std::str::from_utf8(&self.partial) {
            Ok(_) => self.partial.len(),

            Err(e) if e.error_len().is_none() => e.valid_up_to(),
            Err(_) => self.partial.len(),
        };
        let rest = self.partial.split_off(valid);
        let text = String::from_utf8_lossy(&self.partial).into_owned();
        self.partial = rest;
        if !text.is_empty() {
            self.event("o", &text);
        }
    }

    pub fn resize(&mut self, cols: u16, rows: u16) {
        self.event("r", &format!("{cols}x{rows}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_a_playable_cast() {
        let path = std::env::temp_dir().join(format!("zhell-cast-{}.cast", std::process::id()));
        let mut c = Cast::create(&path, 80, 24, Some("demo")).unwrap();
        c.output(b"hi \xe2\x9c");
        c.output(b"\x93 \x1b[1mbold\x1b[0m\r\n");
        c.resize(100, 30);
        drop(c);
        let text = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<serde_json::Value> = text.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        assert_eq!(lines[0]["version"], 2);
        assert_eq!(lines[0]["width"], 80);
        assert_eq!(lines[0]["title"], "demo");

        assert_eq!(lines[1][2], "hi ");
        assert_eq!(lines[2][2], "✓ \u{1b}[1mbold\u{1b}[0m\r\n");
        assert_eq!(lines[3][1], "r");
        assert_eq!(lines[3][2], "100x30");
        let _ = std::fs::remove_file(path);
    }
}
