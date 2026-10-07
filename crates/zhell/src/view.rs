use zhell_proto::{PaneId, TermSize};

use crate::mirror::Mirror;

pub struct PaneView {
    pub id: PaneId,
    pub mirror: Mirror,

    pub grid: TermSize,
    pub title: Option<String>,
    pub cwd: Option<String>,

    pub remote: Option<(String, String)>,

    pub recording: Option<String>,

    pub scroll_acc: f64,

    pub fixed_title: Option<String>,

    pub ports: Vec<u16>,

    pub images: std::collections::HashMap<u32, ImagePixels>,

    pub user_scrolled: bool,
    pub scroll_anim: Option<ScrollAnim>,
}

pub struct ScrollAnim {
    pub shift: f32,

    pub prev_offset: f32,
    pub prev: crate::mirror::Mirror,
    pub last: std::time::Instant,
}

pub struct ImagePixels {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl PaneView {
    pub fn new(id: PaneId, grid: TermSize) -> Self {
        Self { id, mirror: Mirror::new(), grid, title: None, cwd: None, remote: None, recording: None, scroll_acc: 0.0, fixed_title: None, ports: Vec::new(), images: Default::default(), user_scrolled: false, scroll_anim: None }
    }

    pub fn label(&self) -> String {
        if let Some(t) = &self.fixed_title {
            return t.clone();
        }
        if let Some(cmd) = self
            .mirror
            .blocks
            .iter()
            .rev()
            .find(|b| b.state == zhell_proto::BlockState::Running)
            .and_then(|b| b.cmd.as_deref())
        {
            return short_command(cmd.lines().next().unwrap_or(cmd));
        }
        if let Some((host, dir)) = &self.remote {
            let short_host = host.split('.').next().unwrap_or(host);
            return format!("{short_host}: {}", last_component(dir));
        }
        if let Some(t) = self.title.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
            return last_component(strip_user_host(t));
        }
        if let Some(cwd) = &self.cwd {
            return last_component(&short_dir(cwd));
        }
        "shell".into()
    }
}

fn short_command(cmd: &str) -> String {
    let cmd = cmd.trim();
    let (prog, rest) = cmd.split_once(char::is_whitespace).unwrap_or((cmd, ""));
    let prog = prog.rsplit('/').next().filter(|p| !p.is_empty()).unwrap_or(prog);
    if rest.is_empty() { prog.to_owned() } else { format!("{prog} {}", rest.trim_start()) }
}

fn strip_user_host(t: &str) -> &str {
    match t.split_once(':') {
        Some((who, rest)) if who.contains('@') && !who.contains(char::is_whitespace) && !rest.is_empty() => rest.trim(),
        _ => t,
    }
}

fn last_component(t: &str) -> String {
    let looks_like_path = (t.starts_with('/') || t.starts_with("~/")) && !t.contains(char::is_whitespace);
    if !looks_like_path {
        return t.to_owned();
    }
    let trimmed = t.trim_end_matches('/');
    match trimmed.rsplit('/').next() {
        Some(last) if !last.is_empty() => last.to_owned(),
        _ => t.to_owned(),
    }
}

fn short_dir(p: &str) -> String {
    match dirs::home_dir().map(|h| h.display().to_string()) {
        Some(h) if p == h => "~".into(),
        Some(h) if p.starts_with(&format!("{h}/")) => format!("~{}", &p[h.len()..]),
        _ => p.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_user_at_host() {
        assert_eq!(strip_user_host("me@box:~/src"), "~/src");
        assert_eq!(strip_user_host("vim main.rs"), "vim main.rs");
        assert_eq!(strip_user_host("Note: x@y"), "Note: x@y");
        assert_eq!(last_component("/usr/share/doc"), "doc");
        assert_eq!(short_command("/usr/bin/cargo build --release"), "cargo build --release");
        assert_eq!(short_command("ls"), "ls");
        assert_eq!(last_component("~"), "~");
        assert_eq!(last_component("/"), "/");
        assert_eq!(last_component("vim /etc/hosts"), "vim /etc/hosts");
    }
}
