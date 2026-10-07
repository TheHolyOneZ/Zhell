use std::path::Path;

use serde::Deserialize;

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SshProfile {
    pub name: String,
    pub host: String,
    pub user: String,
    pub port: u16,

    pub key: String,

    pub jump: String,

    pub args: Vec<String>,
}

impl SshProfile {
    pub fn ssh_args(&self) -> Vec<String> {
        let mut a = Vec::new();
        if self.port != 0 {
            a.extend(["-p".into(), self.port.to_string()]);
        }
        if !self.key.is_empty() {
            a.extend(["-i".into(), self.key.clone()]);
        }
        if !self.jump.is_empty() {
            a.extend(["-J".into(), self.jump.clone()]);
        }
        a.extend(self.args.iter().cloned());
        let target = if self.user.is_empty() { self.host.clone() } else { format!("{}@{}", self.user, self.host) };
        a.push(target);
        a
    }

    pub fn display_name(&self) -> &str {
        if self.name.is_empty() { &self.host } else { &self.name }
    }
}

pub fn hosts_from_config(path: &Path) -> Vec<String> {
    let mut out = Vec::new();
    collect(path, &mut out, 0);
    out
}

fn collect(path: &Path, out: &mut Vec<String>, depth: u8) {
    let Ok(text) = std::fs::read_to_string(path) else { return };
    let base = path.parent().unwrap_or(Path::new("."));
    for line in text.lines() {
        let line = line.trim();
        let mut words = line.split_whitespace();
        let Some(key) = words.next() else { continue };
        if key.eq_ignore_ascii_case("Host") {
            for h in words {
                if !h.contains(['*', '?', '!']) && !out.iter().any(|o| o == h) {
                    out.push(h.to_owned());
                }
            }
        } else if key.eq_ignore_ascii_case("Include") && depth < 2 {
            for pattern in words {
                let p = if let Some(rest) = pattern.strip_prefix("~/") {
                    dirs::home_dir().map(|h| h.join(rest)).unwrap_or_default()
                } else if Path::new(pattern).is_absolute() {
                    pattern.into()
                } else {
                    base.join(pattern)
                };

                if p.file_name().is_some_and(|n| n == "*") {
                    if let Some(dir) = p.parent()
                        && let Ok(entries) = std::fs::read_dir(dir)
                    {
                        for e in entries.flatten() {
                            collect(&e.path(), out, depth + 1);
                        }
                    }
                } else {
                    collect(&p, out, depth + 1);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn args_from_profile() {
        let p = SshProfile { name: "prod".into(), host: "10.0.0.5".into(), user: "deploy".into(), port: 2222, key: "~/.ssh/prod".into(), jump: "bastion".into(), args: vec!["-A".into()] };
        assert_eq!(p.ssh_args(), vec!["-p", "2222", "-i", "~/.ssh/prod", "-J", "bastion", "-A", "deploy@10.0.0.5"]);
        assert_eq!(SshProfile { host: "box".into(), ..Default::default() }.ssh_args(), vec!["box"]);
    }

    #[test]
    fn hosts_skip_wildcards_and_follow_includes() {
        let d = std::env::temp_dir().join(format!("zhell-ssh-{}", std::process::id()));
        std::fs::create_dir_all(d.join("conf.d")).unwrap();
        std::fs::write(d.join("config"), "Include conf.d/*\nHost *\n  ServerAliveInterval 30\nHost web db.internal\n  User me\n").unwrap();
        std::fs::write(d.join("conf.d/work"), "Host bastion\n").unwrap();
        assert_eq!(hosts_from_config(&d.join("config")), vec!["bastion", "web", "db.internal"]);
        std::fs::remove_dir_all(d).unwrap();
    }
}
