use std::path::{Path, PathBuf};
use std::sync::OnceLock;

const FILES: &[(&str, &str)] = &[
    ("zhell.bash", include_str!("../../../shell-integration/zhell.bash")),
    ("zhell.zsh", include_str!("../../../shell-integration/zhell.zsh")),
    ("zhell.fish", include_str!("../../../shell-integration/zhell.fish")),
    ("zhell.ps1", include_str!("../../../shell-integration/zhell.ps1")),
    ("zsh/.zshenv", include_str!("../../../shell-integration/zsh/.zshenv")),
    ("zsh/.zprofile", include_str!("../../../shell-integration/zsh/.zprofile")),
    ("zsh/.zshrc", include_str!("../../../shell-integration/zsh/.zshrc")),
    ("zsh/.zlogin", include_str!("../../../shell-integration/zsh/.zlogin")),
    ("nushell/vendor/autoload/zhell.nu", include_str!("../../../shell-integration/nushell/vendor/autoload/zhell.nu")),
];

pub fn install_dir() -> Option<&'static Path> {
    static DIR: OnceLock<Option<PathBuf>> = OnceLock::new();
    DIR.get_or_init(|| {
        let base = runtime_base()?;
        let dir = base.join(format!("shell-integration-{}", env!("CARGO_PKG_VERSION")));
        for (rel, text) in FILES {
            let path = dir.join(rel);
            if let Some(parent) = path.parent()
                && let Err(e) = std::fs::create_dir_all(parent)
            {
                log::warn!("shell integration: {e}");
                return None;
            }

            if let Err(e) = std::fs::write(&path, text) {
                log::warn!("shell integration: {e}");
                return None;
            }
        }
        Some(dir)
    })
    .as_deref()
}

fn runtime_base() -> Option<PathBuf> {
    #[cfg(unix)]
    {
        let dir = crate::ipc::runtime_dir();
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new().recursive(true).mode(0o700).create(&dir).ok()?;
        Some(dir)
    }
    #[cfg(windows)]
    {
        dirs::data_local_dir().map(|d| d.join("zhell"))
    }
}

pub fn default_shell() -> Option<String> {
    #[cfg(unix)]
    {
        std::env::var("SHELL").ok().filter(|s| !s.is_empty())
    }
    #[cfg(windows)]
    {
        let in_path = |exe: &str| {
            std::env::var_os("PATH").and_then(|p| std::env::split_paths(&p).map(|d| d.join(exe)).find(|f| f.is_file()))
        };
        let system_ps = std::env::var_os("SystemRoot")
            .map(|r| PathBuf::from(r).join(r"System32\WindowsPowerShell\v1.0\powershell.exe"))
            .filter(|p| p.is_file());
        in_path("pwsh.exe")
            .or_else(|| in_path("powershell.exe"))
            .or(system_ps)
            .or_else(|| std::env::var_os("ComSpec").map(PathBuf::from))
            .map(|p| p.display().to_string())
    }
}

fn powershell_command() -> String {
    use base64::Engine;
    let script = FILES.iter().find(|(n, _)| *n == "zhell.ps1").map_or("", |(_, t)| t);
    let utf16: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    base64::engine::general_purpose::STANDARD.encode(utf16)
}

fn cmd_prompt() -> String {
    let host = zhell_core::prescan::local_hostname();
    format!("$e]133;D$e\\$e]7;file://{host}/$P$e\\$e]133;A$e\\$P$G$e]133;B$e\\")
}

pub struct Launch {
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

pub fn launch_for(program: &str, user_args: &[String]) -> Option<Launch> {
    let dir = install_dir()?;

    let file = program.rsplit(['/', '\\']).next()?.to_ascii_lowercase();
    let name = file.strip_suffix(".exe").unwrap_or(&file).to_owned();
    let p = |rel: &str| dir.join(rel).display().to_string();
    let mut env = vec![("ZHELL_SHELL_INTEGRATION".to_owned(), "1".to_owned())];
    let args = match name.as_str() {
        "bash" if user_args.is_empty() => {
            env.push(("ZHELL_SHELL_LOGIN".into(), "1".into()));
            vec!["--rcfile".into(), p("zhell.bash"), "-i".into()]
        }
        "zsh" => {
            let user = std::env::var("ZDOTDIR").ok().filter(|d| !d.is_empty());
            let home = dirs::home_dir().map(|h| h.display().to_string());
            if let Some(user) = user.or(home) {
                env.push(("ZHELL_USER_ZDOTDIR".into(), user));
            }
            env.push(("ZDOTDIR".into(), p("zsh")));
            if user_args.is_empty() { vec!["-l".into()] } else { Vec::new() }
        }
        "fish" => {
            let mut a = if user_args.is_empty() { vec!["-l".to_owned()] } else { Vec::new() };
            a.extend(["--init-command".into(), format!("source '{}'", p("zhell.fish"))]);
            a
        }
        "pwsh" | "powershell" if user_args.is_empty() => {
            vec!["-NoLogo".into(), "-NoExit".into(), "-EncodedCommand".into(), powershell_command()]
        }
        "cmd" if user_args.is_empty() => {
            env.push(("PROMPT".into(), cmd_prompt()));
            Vec::new()
        }
        "nu" => {
            let existing = std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
            let sep = if cfg!(windows) { ';' } else { ':' };
            env.push(("XDG_DATA_DIRS".into(), format!("{}{sep}{existing}", dir.display())));
            Vec::new()
        }
        _ => return None,
    };
    Some(Launch { args, env })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn powershell_runs_the_script_as_an_encoded_command() {
        let l = launch_for(r"C:\Program Files\PowerShell\7\pwsh.exe", &[]).unwrap();
        assert_eq!(l.args[..3], ["-NoLogo", "-NoExit", "-EncodedCommand"]);
        use base64::Engine;
        let bytes = base64::engine::general_purpose::STANDARD.decode(&l.args[3]).unwrap();
        let utf16: Vec<u16> = bytes.chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        let script = String::from_utf16(&utf16).unwrap();
        assert!(script.contains("function global:prompt") && script.contains("133;A"));

        assert!(launch_for("powershell.exe", &["-File".into(), "x.ps1".into()]).is_none());
    }

    #[test]
    fn cmd_gets_marks_in_its_prompt() {
        let l = launch_for(r"C:\Windows\System32\cmd.exe", &[]).unwrap();
        let prompt = &l.env.iter().find(|(k, _)| k == "PROMPT").unwrap().1;
        assert!(prompt.starts_with("$e]133;D$e\\") && prompt.contains("$e]133;A$e\\$P$G$e]133;B$e\\"), "{prompt}");
        assert!(launch_for("cmd.exe", &["/c".into(), "dir".into()]).is_none());
    }

    #[test]
    fn bash_gets_rcfile_only_without_user_args() {
        let l = launch_for("/usr/bin/bash", &[]).unwrap();
        assert_eq!(l.args[0], "--rcfile");
        assert!(Path::new(&l.args[1]).exists());

        assert!(launch_for("bash", &["-c".into(), "ls".into()]).is_none());
        assert!(launch_for("/usr/bin/python3", &[]).is_none());
    }

    #[test]
    fn zsh_gets_zdotdir_shim() {
        let l = launch_for("zsh", &[]).unwrap();
        let zdotdir = &l.env.iter().find(|(k, _)| k == "ZDOTDIR").unwrap().1;
        assert!(Path::new(zdotdir).join(".zshrc").exists());
    }
}
