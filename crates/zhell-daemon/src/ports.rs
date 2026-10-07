use std::collections::{BTreeMap, BTreeSet};

#[cfg(target_os = "linux")]
pub fn listening_by_session(sessions: &[u32]) -> BTreeMap<u32, BTreeMap<u16, u32>> {
    let mut out: BTreeMap<u32, BTreeMap<u16, u32>> = BTreeMap::new();
    let trees: Vec<(u32, Vec<u32>)> = sessions.iter().map(|&s| (s, descendants(s))).filter(|(_, d)| !d.is_empty()).collect();
    if trees.is_empty() {
        return out;
    }

    let mut listen: BTreeMap<u64, u16> = BTreeMap::new();
    for file in ["/proc/net/tcp", "/proc/net/tcp6"] {
        let Ok(text) = std::fs::read_to_string(file) else { continue };
        for line in text.lines().skip(1) {
            let f: Vec<&str> = line.split_whitespace().collect();
            if f.len() < 10 || f[3] != "0A" {
                continue;
            }
            let port = f[1].rsplit(':').next().and_then(|p| u16::from_str_radix(p, 16).ok());
            if let (Some(port), Ok(inode)) = (port, f[9].parse::<u64>()) {
                listen.insert(inode, port);
            }
        }
    }
    if listen.is_empty() {
        return out;
    }
    for (sid, pids) in trees {
        for pid in pids {
            let Ok(fds) = std::fs::read_dir(format!("/proc/{pid}/fd")) else { continue };
            for fd in fds.flatten() {
                let Ok(target) = std::fs::read_link(fd.path()) else { continue };
                let t = target.to_string_lossy();
                if let Some(inode) = t.strip_prefix("socket:[").and_then(|s| s.strip_suffix(']')).and_then(|s| s.parse::<u64>().ok())
                    && let Some(port) = listen.get(&inode)
                {
                    out.entry(sid).or_default().insert(*port, pid);
                }
            }
        }
    }
    out
}

#[cfg(target_os = "linux")]
fn descendants(pid: u32) -> Vec<u32> {
    let mut out = Vec::new();
    let mut todo = vec![pid];
    while let Some(p) = todo.pop() {
        let Ok(tasks) = std::fs::read_dir(format!("/proc/{p}/task")) else { continue };
        for t in tasks.flatten() {
            let Ok(children) = std::fs::read_to_string(t.path().join("children")) else { continue };
            for c in children.split_whitespace().filter_map(|c| c.parse::<u32>().ok()) {
                if !out.contains(&c) && out.len() < 4096 {
                    out.push(c);
                    todo.push(c);
                }
            }
        }
    }
    out
}

#[cfg(not(target_os = "linux"))]
pub fn listening_by_session(_sessions: &[u32]) -> BTreeMap<u32, BTreeMap<u16, u32>> {
    BTreeMap::new()
}

pub fn ports(map: &BTreeMap<u16, u32>) -> BTreeSet<u16> {
    map.keys().copied().collect()
}

pub fn terminate(pid: u32) -> bool {
    #[cfg(unix)]
    {
        std::process::Command::new("kill").args(["-TERM", &pid.to_string()]).status().is_ok_and(|s| s.success())
    }
    #[cfg(windows)]
    {
        std::process::Command::new("taskkill").args(["/PID", &pid.to_string()]).status().is_ok_and(|s| s.success())
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn finds_a_listening_child() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        let mut child = std::process::Command::new("python3")
            .args(["-c", "import socket,time,sys; s=socket.socket(); s.bind(('127.0.0.1',0)); s.listen(); print(s.getsockname()[1], flush=True); time.sleep(10)"])
            .stdout(std::process::Stdio::piped())
            .spawn();
        let Ok(child) = child.as_mut() else { return };
        let mut line = String::new();
        std::io::BufRead::read_line(&mut std::io::BufReader::new(child.stdout.as_mut().unwrap()), &mut line).unwrap();
        let child_port: u16 = line.trim().parse().unwrap();
        let me = std::process::id();
        let found = listening_by_session(&[me]);
        let mine = found.get(&me).cloned().unwrap_or_default();
        assert_eq!(mine.get(&child_port), Some(&child.id()), "{found:?}");

        assert!(!mine.contains_key(&port));
        let _ = child.kill();
        let _ = child.wait();
    }
}
