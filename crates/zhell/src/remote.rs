const BASH: &str = include_str!("../../../shell-integration/zhell.bash");
const ZSH: &str = include_str!("../../../shell-integration/zhell.zsh");
const FISH: &str = include_str!("../../../shell-integration/zhell.fish");

const MARK: &str = "zhell-integration";

fn bash_core() -> String {
    let mut out = String::new();
    let mut skipping = false;
    for line in BASH.lines() {
        if line.starts_with("if [[ -n \"${ZHELL_SHELL_LOGIN:-}\" ]]; then") {
            skipping = true;
            continue;
        }
        if skipping {
            if line == "fi" {
                skipping = false;
            }
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

pub fn installer_script() -> String {
    let heredoc = |file: &str, body: &str| format!("cat > \"$d/{file}\" <<'ZHELL_END_OF_SCRIPT'\n{}\nZHELL_END_OF_SCRIPT\n", body.trim_end());

    let source = |file: &str| format!("[ \\\"\\$TERM\\\" != dumb ] && [ -r \\\"\\$HOME/.local/share/zhell/{file}\\\" ] && . \\\"\\$HOME/.local/share/zhell/{file}\\\"  # {MARK}");
    format!(
        r#"d="$HOME/.local/share/zhell"
mkdir -p "$d" || exit 1
{bash}{zsh}{fish}added=""
add() {{
    if [ -f "$1" ] && grep -q '{MARK}' "$1"; then return 0; fi
    printf '\n%s\n' "$2" >> "$1" && added="$added ${{1##*/}}"
}}
add "$HOME/.bashrc" "{bash_line}"
if command -v zsh >/dev/null 2>&1 || [ -f "${{ZDOTDIR:-$HOME}}/.zshrc" ]; then
    add "${{ZDOTDIR:-$HOME}}/.zshrc" "{zsh_line}"
fi
if command -v fish >/dev/null 2>&1; then
    mkdir -p "$HOME/.config/fish/conf.d" && cp "$d/zhell.fish" "$HOME/.config/fish/conf.d/zhell.fish" && added="$added fish"
fi
printf '\033[32m✓\033[0m Zhell shell integration installed on %s%s. Run  exec $SHELL  or reconnect to start it.\n' "$(uname -n)" "${{added:+ (${{added# }})}}"
printf '  Remove it with  rm -r ~/.local/share/zhell ~/.config/fish/conf.d/zhell.fish  and the lines marked {MARK}.\n'
"#,
        bash = heredoc("zhell.bash", &bash_core()),
        zsh = heredoc("zhell.zsh", ZSH),
        fish = heredoc("zhell.fish", FISH),
        bash_line = source("zhell.bash"),
        zsh_line = source("zhell.zsh"),
    )
}

pub fn installer_command() -> String {
    " stty -echo; printf '\\033]6973;ready\\007'; base64 -d | gzip -dc | sh; stty echo\r".to_owned()
}

pub fn installer_payload() -> String {
    let b64 = base64(&gzip(installer_script().as_bytes()));
    let mut out = String::with_capacity(b64.len() + b64.len() / 76 + 4);
    for chunk in b64.as_bytes().chunks(76) {
        out.push_str(std::str::from_utf8(chunk).unwrap_or_default());
        out.push('\n');
    }

    out.push('\x04');
    out
}

fn gzip(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 3];
    out.extend(miniz_oxide::deflate::compress_to_vec(data, 9));
    out.extend(crc32(data).to_le_bytes());
    out.extend((data.len() as u32).to_le_bytes());
    out
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= c.len() {
                out.push(T[(n >> (18 - 6 * i)) as usize & 63] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bash_core_drops_the_rc_preamble() {
        let core = bash_core();
        assert!(!core.contains(". ~/.bashrc") && !core.contains("ZHELL_SHELL_LOGIN"));
        assert!(core.contains("__zhell_precmd") && core.contains("__zhell_loaded=1"));
    }

    #[test]
    fn crc_and_base64() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(base64(b"Zhell!"), "WmhlbGwh");
        assert_eq!(base64(b"ab"), "YWI=");
    }

    #[test]
    #[cfg(unix)]
    fn installs_into_a_home_directory() {
        use std::process::Command;
        let home = std::env::temp_dir().join(format!("zhell-remote-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(home.join(".bashrc"), "# mine\n").unwrap();

        let script = installer_command().replace("stty -echo; ", "").replace("; stty echo", "").replace("printf '\\033]6973;ready\\007'; ", "");
        let payload = installer_payload().replace('\x04', "");
        for _ in 0..2 {
            use std::io::Write;
            let mut child = Command::new("sh")
                .arg("-c")
                .arg(script.trim())
                .env("HOME", &home)
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .unwrap();
            child.stdin.take().unwrap().write_all(payload.as_bytes()).unwrap();
            let out = child.wait_with_output().unwrap();
            assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
            assert!(String::from_utf8_lossy(&out.stdout).contains("installed"));
        }
        let rc = std::fs::read_to_string(home.join(".bashrc")).unwrap();
        assert!(rc.starts_with("# mine\n"));
        assert_eq!(rc.matches(MARK).count(), 1, "added once: {rc}");
        assert!(rc.contains("[ \"$TERM\" != dumb ] && [ -r \"$HOME/.local/share/zhell/zhell.bash\" ]"), "{rc}");
        let installed = std::fs::read_to_string(home.join(".local/share/zhell/zhell.bash")).unwrap();
        assert_eq!(installed.trim_end(), bash_core().trim_end());

        if Command::new("bash").arg("--version").output().is_ok() {
            let out = Command::new("bash").args(["-i", "-c", "type -t __zhell_precmd"]).env("HOME", &home).env("TERM", "xterm").output().unwrap();
            assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "function", "{}", String::from_utf8_lossy(&out.stderr));
        }
        let _ = std::fs::remove_dir_all(&home);
    }
}
