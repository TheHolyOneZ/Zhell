use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crossbeam_channel::{Receiver, RecvTimeoutError, Sender, unbounded};
use zhell_history::{History, NewBlock};
use zhell_proto::{HistoryQuery, ServerMsg};

use crate::Outbox;

pub enum HistoryCmd {
    Insert(NewBlock),
    Search { req: u32, query: HistoryQuery, reply: Outbox },
    Get { req: u32, id: i64, reply: Outbox },
    Star { id: i64, starred: bool },
    Note { id: i64, note: Option<String> },
    Template { id: i64, template: Option<String> },
    Forget { id: i64 },
}

#[derive(Clone, Debug)]
pub struct HistorySettings {
    pub path: PathBuf,
    pub max_bytes: u64,
    pub max_age_days: Option<u32>,

    pub exclude_dirs: Vec<String>,
    pub encrypt: bool,
}

impl HistorySettings {
    pub fn excluded(&self, cwd: Option<&str>) -> bool {
        let Some(cwd) = cwd else { return false };
        self.exclude_dirs.iter().any(|d| {
            let d = d.trim_end_matches('/');
            cwd == d || cwd.strip_prefix(d).is_some_and(|rest| rest.starts_with('/'))
        })
    }
}

impl HistorySettings {
    pub fn from_config(c: &zhell_core::config::HistoryConfig) -> Option<Self> {
        if !c.enabled {
            return None;
        }
        let path = if c.path.is_empty() { default_path()? } else { expand_home(&c.path) };
        Some(Self {
            path,
            max_bytes: c.max_size_mb.saturating_mul(1024 * 1024),
            max_age_days: (c.max_age_days > 0).then_some(c.max_age_days),
            exclude_dirs: c.exclude_dirs.iter().map(|d| expand_home(d).display().to_string()).collect(),
            encrypt: c.encrypt,
        })
    }
}

fn expand_home(p: &str) -> PathBuf {
    match (p.strip_prefix("~/"), dirs::home_dir()) {
        (Some(rest), Some(home)) => home.join(rest),
        _ => PathBuf::from(p),
    }
}

pub fn default_path() -> Option<PathBuf> {
    dirs::state_dir().or_else(dirs::data_local_dir).map(|d| d.join("zhell").join("history.db"))
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

pub fn start(settings: HistorySettings) -> Option<Sender<HistoryCmd>> {
    let db = match open_db(&settings) {
        Ok(db) => db,
        Err(e) => {
            log::error!("history disabled: {e}");
            return None;
        }
    };
    let (tx, rx) = unbounded();
    thread::Builder::new().name("history".into()).spawn(move || run(db, rx, settings)).ok()?;
    Some(tx)
}

fn open_db(settings: &HistorySettings) -> zhell_history::Result<History> {
    if settings.encrypt {
        let key = zhell_history::history_key()?;
        return History::open_with_key(&settings.path, Some(&key));
    }
    match History::open(&settings.path) {
        Err(zhell_history::Error::WrongKey) => {
            let key = zhell_history::existing_history_key().ok_or(zhell_history::Error::WrongKey)?;
            zhell_history::decrypt_in_place(&settings.path, &key)?;
            History::open(&settings.path)
        }
        other => other,
    }
}

fn run(mut db: History, rx: Receiver<HistoryCmd>, settings: HistorySettings) {
    const RETENTION_EVERY: Duration = Duration::from_secs(3600);
    let mut next_retention = Instant::now() + Duration::from_secs(30);
    let mut pending = Vec::new();
    loop {
        let cmd = match rx.recv_timeout(Duration::from_secs(60)) {
            Ok(c) => Some(c),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => break,
        };

        let mut cmds: Vec<HistoryCmd> = cmd.into_iter().collect();
        cmds.extend(rx.try_iter());
        for cmd in cmds {
            match cmd {
                HistoryCmd::Insert(b) => pending.push(b),
                other => {
                    flush(&mut db, &mut pending);
                    handle(&db, other);
                }
            }
        }
        flush(&mut db, &mut pending);
        if Instant::now() >= next_retention {
            next_retention = Instant::now() + RETENTION_EVERY;
            match db.enforce_retention(settings.max_bytes, settings.max_age_days, now_ms()) {
                Ok(0) => {}
                Ok(n) => log::info!("history retention removed {n} old entries"),
                Err(e) => log::warn!("history retention: {e}"),
            }
        }
    }
    flush(&mut db, &mut pending);
}

fn flush(db: &mut History, pending: &mut Vec<NewBlock>) {
    if pending.is_empty() {
        return;
    }
    if let Err(e) = db.insert(pending) {
        log::warn!("history insert: {e}");
    }
    pending.clear();
}

fn handle(db: &History, cmd: HistoryCmd) {
    let result = match cmd {
        HistoryCmd::Insert(_) => Ok(()),
        HistoryCmd::Search { req, query, reply } => db.search(&query).map(|hits| reply.send(ServerMsg::HistoryResults { req, hits })),
        HistoryCmd::Get { req, id, reply } => db.get(id).map(|e| {
            let entry = e.map(|e| (e.hit, e.output));
            reply.send(ServerMsg::HistoryEntry { req, entry });
        }),
        HistoryCmd::Star { id, starred } => db.set_starred(id, starred),
        HistoryCmd::Note { id, note } => db.set_note(id, note.as_deref()),
        HistoryCmd::Template { id, template } => db.set_template(id, template.as_deref()),
        HistoryCmd::Forget { id } => db.forget(id),
    };
    if let Err(e) = result {
        log::warn!("{e}");
    }
}

pub fn hostname() -> String {
    #[cfg(unix)]
    {
        std::fs::read_to_string("/etc/hostname")
            .ok()
            .map(|h| h.trim().to_owned())
            .filter(|h| !h.is_empty())
            .or_else(|| std::env::var("HOSTNAME").ok())
            .unwrap_or_else(|| "localhost".into())
    }
    #[cfg(windows)]
    {
        std::env::var("COMPUTERNAME").unwrap_or_else(|_| "localhost".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exclusions_are_directory_prefixes() {
        let s = HistorySettings {
            path: PathBuf::new(),
            max_bytes: 0,
            max_age_days: None,
            exclude_dirs: vec!["/home/z/secret/".into()],
            encrypt: false,
        };
        assert!(s.excluded(Some("/home/z/secret")));
        assert!(s.excluded(Some("/home/z/secret/deep")));
        assert!(!s.excluded(Some("/home/z/secretary")));
        assert!(!s.excluded(None));
    }
}
