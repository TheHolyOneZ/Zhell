use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use zhell_proto::{PaneId, TermSize};

pub const MAX_LINES: usize = 2000;
const FORMAT_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PaneSnap {
    pub id: PaneId,
    pub cwd: Option<String>,
    pub title: Option<String>,
    pub size: TermSize,

    pub lines: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Group {
    pub layout: Vec<u8>,
    pub panes: Vec<PaneSnap>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub version: u32,

    pub saved_at: u64,
    pub groups: Vec<Group>,
}

impl Snapshot {
    pub fn new(groups: Vec<Group>) -> Self {
        let saved_at = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
        Self { version: FORMAT_VERSION, saved_at, groups }
    }
}

pub fn default_path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("ZHELL_SNAPSHOT") {
        return Some(PathBuf::from(p));
    }
    dirs::state_dir().or_else(dirs::data_local_dir).map(|d| d.join("zhell").join("sessions.bin"))
}

pub fn save(path: &Path, snap: &Snapshot) -> std::io::Result<()> {
    let bytes = bincode::serde::encode_to_vec(snap, bincode::config::standard())
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(&tmp)?;
    f.write_all(&bytes)?;
    f.sync_all()?;
    drop(f);
    std::fs::rename(&tmp, path)
}

pub fn load(path: &Path) -> Option<Snapshot> {
    let bytes = std::fs::read(path).ok()?;
    let (snap, _): (Snapshot, _) = bincode::serde::decode_from_slice(&bytes, bincode::config::standard()).ok()?;
    (snap.version == FORMAT_VERSION).then_some(snap)
}

pub fn remove(path: &Path) {
    let _ = std::fs::remove_file(path);
}

pub fn restored_banner(snap_time: u64) -> String {
    let ago = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs()).saturating_sub(snap_time);
    let when = match ago {
        0..=119 => "just now".to_owned(),
        120..=7199 => format!("{} min ago", ago / 60),
        7200..=172_799 => format!("{} h ago", ago / 3600),
        _ => format!("{} days ago", ago / 86_400),
    };
    format!("\x1b[2m── session restored · saved {when} · processes were not restored ──\x1b[0m")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_permissions() {
        let dir = std::env::temp_dir().join(format!("zhell-snap-{}", std::process::id()));
        let path = dir.join("s.bin");
        let snap = Snapshot::new(vec![Group {
            layout: vec![1, 2, 3],
            panes: vec![PaneSnap {
                id: PaneId(4),
                cwd: Some("/tmp".into()),
                title: Some("t".into()),
                size: TermSize { cols: 80, rows: 24, cell_width: 9, cell_height: 19 },
                lines: vec!["$ ls".into(), "a b c".into()],
            }],
        }]);
        save(&path, &snap).unwrap();
        assert_eq!(load(&path), Some(snap));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        }
        std::fs::write(&path, b"garbage").unwrap();
        assert_eq!(load(&path), None);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
