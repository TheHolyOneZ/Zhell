use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Task {
    pub name: String,
    pub command: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Project {
    pub root: PathBuf,

    pub kinds: Vec<String>,
    pub tasks: Vec<Task>,

    pub primary: Option<Task>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceTab {
    pub title: String,

    #[serde(default)]
    pub cwd: String,

    #[serde(default)]
    pub command: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Workspace {
    #[serde(rename = "tab")]
    pub tabs: Vec<WorkspaceTab>,
}

pub const WORKSPACE_FILE: &str = ".zhell/workspace.toml";

fn read(path: &Path) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

fn node_runner(dir: &Path) -> &'static str {
    if dir.join("pnpm-lock.yaml").exists() {
        "pnpm"
    } else if dir.join("yarn.lock").exists() {
        "yarn"
    } else if dir.join("bun.lockb").exists() || dir.join("bun.lock").exists() {
        "bun"
    } else {
        "npm"
    }
}

fn targets(text: &str, just: bool) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        if line.starts_with([' ', '\t', '#', '.', '%', '@']) || line.contains(":=") || line.contains("?=") {
            continue;
        }
        let Some((head, _)) = line.split_once(':') else { continue };
        let name = if just { head.split_whitespace().next().unwrap_or("") } else { head.trim() };
        let valid = !name.is_empty()
            && !name.contains(['$', '/', '=', ' '])
            && name.chars().all(|c| c.is_alphanumeric() || "-_".contains(c));
        if valid && !out.iter().any(|t| t == name) {
            out.push(name.to_owned());
        }
    }
    out
}

pub fn detect(dir: &Path) -> Option<Project> {
    let mut kinds = Vec::new();
    let mut tasks: Vec<Task> = Vec::new();
    let mut primary: Option<Task> = None;
    let task = |name: &str, command: String| Task { name: name.to_owned(), command };

    let tauri = dir.join("src-tauri/tauri.conf.json").exists() || dir.join("tauri.conf.json").exists();
    if let Some(pkg) = read(&dir.join("package.json")) {
        let runner = node_runner(dir);
        kinds.push(format!("Node ({runner})"));
        let json: serde_json::Value = serde_json::from_str(&pkg).unwrap_or_default();
        if let Some(scripts) = json.get("scripts").and_then(|s| s.as_object()) {
            for name in scripts.keys() {
                let cmd = match runner {
                    "npm" => format!("npm run {name}"),
                    r => format!("{r} {name}"),
                };
                tasks.push(task(name, cmd));
            }
            if tauri && scripts.contains_key("tauri") {
                primary = Some(task("tauri dev", format!("{} tauri dev", if runner == "npm" { "npm run" } else { runner })));
            }
            if primary.is_none() {
                primary = ["dev", "start", "serve"]
                    .iter()
                    .find_map(|n| tasks.iter().find(|t| t.name == *n).cloned());
            }
        }
    }
    if tauri {
        kinds.insert(0, "Tauri".into());
        if primary.is_none() {
            primary = Some(task("tauri dev", "cargo tauri dev".into()));
        }
    }
    if dir.join("Cargo.toml").exists() {
        kinds.push("Rust".into());
        for (n, c) in [("cargo run", "cargo run"), ("cargo test", "cargo test"), ("cargo build", "cargo build"), ("cargo clippy", "cargo clippy")] {
            tasks.push(task(n, c.into()));
        }
        if primary.is_none() && !tauri {
            primary = Some(task("cargo run", "cargo run".into()));
        }
    }
    if dir.join("go.mod").exists() {
        kinds.push("Go".into());
        tasks.push(task("go run", "go run .".into()));
        tasks.push(task("go test", "go test ./...".into()));
        primary.get_or_insert_with(|| task("go run", "go run .".into()));
    }
    for makefile in ["Makefile", "makefile", "GNUmakefile"] {
        if let Some(text) = read(&dir.join(makefile)) {
            kinds.push("Make".into());
            tasks.extend(targets(&text, false).into_iter().map(|t| task(&format!("make {t}"), format!("make {t}"))));
            break;
        }
    }
    for justfile in ["justfile", "Justfile", ".justfile"] {
        if let Some(text) = read(&dir.join(justfile)) {
            kinds.push("just".into());
            tasks.extend(targets(&text, true).into_iter().map(|t| task(&format!("just {t}"), format!("just {t}"))));
            break;
        }
    }
    if ["docker-compose.yml", "docker-compose.yaml", "compose.yml", "compose.yaml"].iter().any(|f| dir.join(f).exists()) {
        kinds.push("Docker Compose".into());
        tasks.push(task("compose up", "docker compose up".into()));
        tasks.push(task("compose down", "docker compose down".into()));
    }
    if kinds.is_empty() && !dir.join(WORKSPACE_FILE).exists() {
        return None;
    }
    Some(Project { root: dir.to_owned(), kinds, tasks, primary })
}

impl Project {
    pub fn workspace(&self) -> Workspace {
        if let Some(ws) = load_workspace(&self.root) {
            return ws;
        }
        let mut tabs = Vec::new();
        if let Some(p) = &self.primary {
            tabs.push(WorkspaceTab { title: "dev".into(), cwd: String::new(), command: p.command.clone() });
        }
        if self.root.join(".git").exists() {
            tabs.push(WorkspaceTab { title: "git".into(), cwd: String::new(), command: "git status".into() });
        }
        tabs.push(WorkspaceTab { title: "shell".into(), cwd: String::new(), command: String::new() });
        Workspace { tabs }
    }
}

pub fn load_workspace(root: &Path) -> Option<Workspace> {
    let text = read(&root.join(WORKSPACE_FILE))?;
    match toml::from_str(&text) {
        Ok(ws) => Some(ws),
        Err(e) => {
            log::warn!("{}: {e}", root.join(WORKSPACE_FILE).display());
            None
        }
    }
}

pub fn save_workspace(root: &Path, ws: &Workspace) -> std::io::Result<PathBuf> {
    let path = root.join(WORKSPACE_FILE);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let body = toml::to_string_pretty(ws).map_err(std::io::Error::other)?;
    std::fs::write(&path, format!("# Zhell workspace: tabs opened by \"Open project workspace\".\n# Commit this file to share the setup.\n\n{body}"))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str, files: &[(&str, &str)]) -> PathBuf {
        let d = std::env::temp_dir().join(format!("zhell-proj-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        for (f, body) in files {
            let p = d.join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, body).unwrap();
        }
        d
    }

    #[test]
    fn tauri_with_pnpm() {
        let d = dir(
            "tauri",
            &[
                ("package.json", r#"{"scripts": {"dev": "vite", "build": "vite build", "tauri": "tauri"}}"#),
                ("pnpm-lock.yaml", ""),
                ("src-tauri/tauri.conf.json", "{}"),
                ("src-tauri/Cargo.toml", ""),
                (".git/HEAD", ""),
            ],
        );
        let p = detect(&d).unwrap();
        assert_eq!(p.kinds, vec!["Tauri", "Node (pnpm)"]);
        assert_eq!(p.primary.as_ref().unwrap().command, "pnpm tauri dev");
        assert!(p.tasks.iter().any(|t| t.command == "pnpm build"));
        let ws = p.workspace();
        assert_eq!(ws.tabs.iter().map(|t| t.title.as_str()).collect::<Vec<_>>(), vec!["dev", "git", "shell"]);
    }

    #[test]
    fn rust_make_just_compose() {
        let d = dir(
            "mixed",
            &[
                ("Cargo.toml", "[package]"),
                ("Makefile", "VAR := 1\n.PHONY: all\nall: build\nbuild:\n\tcargo build\n%.o: %.c\n"),
                ("justfile", "set shell := [\"bash\"]\nrelease version:\n  echo {{version}}\n"),
                ("compose.yaml", ""),
            ],
        );
        let p = detect(&d).unwrap();
        assert_eq!(p.primary.as_ref().unwrap().command, "cargo run");
        let cmds: Vec<&str> = p.tasks.iter().map(|t| t.command.as_str()).collect();
        assert!(cmds.contains(&"make all") && cmds.contains(&"make build"));
        assert!(!cmds.iter().any(|c| c.contains(".PHONY") || c.contains("%")));
        assert!(cmds.contains(&"just release"));
        assert!(cmds.contains(&"docker compose up"));
    }

    #[test]
    fn plain_dir_is_not_a_project() {
        assert!(detect(&dir("plain", &[("notes.txt", "")])).is_none());
    }

    #[test]
    fn workspace_roundtrip_overrides_proposal() {
        let d = dir("ws", &[("Cargo.toml", "")]);
        let p = detect(&d).unwrap();
        let ws = Workspace { tabs: vec![WorkspaceTab { title: "watch".into(), cwd: "src".into(), command: "cargo watch".into() }] };
        save_workspace(&d, &ws).unwrap();
        assert_eq!(p.workspace(), ws);
    }
}
