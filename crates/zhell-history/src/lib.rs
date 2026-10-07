use std::path::Path;

use rusqlite::{Connection, OptionalExtension, params};
use zhell_proto::{HistoryHit, HistoryQuery};

pub const MAX_OUTPUT_BYTES: usize = 2 * 1024 * 1024;
const SCHEMA_VERSION: i32 = 2;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("history database: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("history database: {0}")]
    Io(#[from] std::io::Error),
    #[error("history encryption key: {0}")]
    Key(String),
    #[error("history database can't be opened with this key (wrong key, or encrypted/plain mismatch)")]
    WrongKey,
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewBlock {
    pub host: String,
    pub cwd: Option<String>,
    pub cmd: String,
    pub exit_code: Option<i32>,
    pub started_ms: u64,
    pub ended_ms: u64,
    pub output: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub hit: HistoryHit,
    pub output: String,
    pub truncated: bool,
}

pub struct History {
    conn: Connection,
}

fn apply_key(conn: &Connection, key: Option<&str>) -> Result<()> {
    if let Some(k) = key {
        if k.len() != 64 || !k.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(Error::Key("malformed key".into()));
        }

        conn.execute_batch(&format!("PRAGMA key = \"x'{k}'\";"))?;
    }
    Ok(())
}

fn opens_with(path: &Path, key: Option<&str>) -> bool {
    let Ok(conn) = Connection::open(path) else { return false };
    apply_key(&conn, key).is_ok() && conn.query_row("SELECT count(*) FROM sqlite_master", [], |r| r.get::<_, i64>(0)).is_ok()
}

fn convert(path: &Path, from: Option<&str>, to: Option<&str>) -> Result<()> {
    let tmp = path.with_extension("db.converting");
    let _ = std::fs::remove_file(&tmp);
    {
        let conn = Connection::open(path)?;
        apply_key(&conn, from)?;
        let to_key = to.map_or_else(String::new, |k| format!("x'{k}'"));
        conn.execute("ATTACH DATABASE ?1 AS converted KEY ?2", params![tmp.display().to_string(), to_key])?;
        conn.query_row("SELECT sqlcipher_export('converted')", [], |_| Ok(()))?;
        let version: i32 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
        conn.execute_batch(&format!("PRAGMA converted.user_version = {version}; DETACH DATABASE converted;"))?;
    }

    for ext in ["db-wal", "db-shm"] {
        let _ = std::fs::remove_file(path.with_extension(ext));
    }
    std::fs::rename(&tmp, path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    log::info!("history database {}", if to.is_some() { "encrypted" } else { "decrypted" });
    Ok(())
}

pub fn existing_history_key() -> Option<String> {
    keyring::Entry::new("zhell", "history-database-key").ok()?.get_password().ok()
}

pub fn decrypt_in_place(path: &Path, key: &str) -> Result<()> {
    if !opens_with(path, Some(key)) {
        return Err(Error::WrongKey);
    }
    convert(path, Some(key), None)
}

pub fn history_key() -> Result<String> {
    let entry = keyring::Entry::new("zhell", "history-database-key").map_err(|e| Error::Key(e.to_string()))?;
    match entry.get_password() {
        Ok(k) => Ok(k),
        Err(keyring::Error::NoEntry) => {
            let mut bytes = [0u8; 32];
            getrandom::fill(&mut bytes).map_err(|e| Error::Key(e.to_string()))?;
            let key: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
            entry.set_password(&key).map_err(|e| Error::Key(e.to_string()))?;
            Ok(key)
        }
        Err(e) => Err(Error::Key(e.to_string())),
    }
}

fn cap_output(s: &str) -> (String, bool) {
    if s.len() <= MAX_OUTPUT_BYTES {
        return (s.to_owned(), false);
    }
    let half = MAX_OUTPUT_BYTES / 2;
    let mut head = half;
    while !s.is_char_boundary(head) {
        head -= 1;
    }
    let mut tail = s.len() - half;
    while !s.is_char_boundary(tail) {
        tail += 1;
    }
    (format!("{}\n… {} bytes omitted …\n{}", &s[..head], tail - head, &s[tail..]), true)
}

fn fts_query(text: &str) -> Option<String> {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.is_empty() {
        return None;
    }
    let n = words.len();
    Some(
        words
            .iter()
            .enumerate()
            .map(|(i, w)| {
                let q = format!("\"{}\"", w.replace('"', "\"\""));
                if i + 1 == n { format!("{q}*") } else { q }
            })
            .collect::<Vec<_>>()
            .join(" "),
    )
}

impl History {
    pub fn open(path: &Path) -> Result<Self> {
        Self::open_with_key(path, None)
    }

    pub fn open_with_key(path: &Path, key: Option<&str>) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        if path.exists() && !opens_with(path, key) {
            let other: Option<&str> = if key.is_some() { None } else { return Err(Error::WrongKey) };
            if !opens_with(path, other) {
                return Err(Error::WrongKey);
            }
            convert(path, other, key)?;
        }
        #[cfg(unix)]
        let existed = path.exists();
        let conn = Connection::open(path)?;
        apply_key(&conn, key)?;
        #[cfg(unix)]
        if !existed {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        }
        Self::init(conn)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        let version: i32 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version == 0 {
            conn.pragma_update(None, "auto_vacuum", "INCREMENTAL")?;
            conn.execute_batch(
                "
                CREATE TABLE blocks (
                    id          INTEGER PRIMARY KEY,
                    host        TEXT NOT NULL,
                    cwd         TEXT,
                    cmd         TEXT NOT NULL,
                    exit_code   INTEGER,
                    started_ms  INTEGER NOT NULL,
                    ended_ms    INTEGER NOT NULL,
                    output      TEXT NOT NULL,
                    truncated   INTEGER NOT NULL DEFAULT 0,
                    starred     INTEGER NOT NULL DEFAULT 0,
                    note        TEXT
                );
                CREATE INDEX blocks_started ON blocks(started_ms);
                CREATE VIRTUAL TABLE blocks_fts USING fts5(
                    cmd, output, cwd,
                    content='blocks', content_rowid='id',
                    tokenize='unicode61 remove_diacritics 2'
                );
                CREATE TRIGGER blocks_ai AFTER INSERT ON blocks BEGIN
                    INSERT INTO blocks_fts(rowid, cmd, output, cwd) VALUES (new.id, new.cmd, new.output, new.cwd);
                END;
                CREATE TRIGGER blocks_ad AFTER DELETE ON blocks BEGIN
                    INSERT INTO blocks_fts(blocks_fts, rowid, cmd, output, cwd) VALUES ('delete', old.id, old.cmd, old.output, old.cwd);
                END;
                ",
            )?;
            conn.pragma_update(None, "user_version", 1)?;
        }
        let version: i32 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version < 2 {
            conn.execute_batch("ALTER TABLE blocks ADD COLUMN template TEXT;")?;
            conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        }
        Ok(Self { conn })
    }

    pub fn insert(&mut self, blocks: &[NewBlock]) -> Result<Vec<i64>> {
        let tx = self.conn.transaction()?;
        let mut ids = Vec::with_capacity(blocks.len());
        {
            let mut stmt = tx.prepare_cached(
                "INSERT INTO blocks (host, cwd, cmd, exit_code, started_ms, ended_ms, output, truncated)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            )?;
            for b in blocks {
                let cmd = zhell_secrets::redact(&b.cmd);
                let (output, truncated) = cap_output(&zhell_secrets::redact(&b.output));
                stmt.execute(params![
                    b.host,
                    b.cwd,
                    cmd,
                    b.exit_code,
                    b.started_ms as i64,
                    b.ended_ms as i64,
                    output,
                    truncated
                ])?;
                ids.push(tx.last_insert_rowid());
            }
        }
        tx.commit()?;
        Ok(ids)
    }

    pub fn search(&self, q: &HistoryQuery) -> Result<Vec<HistoryHit>> {
        let limit = if q.limit == 0 { 200 } else { q.limit.min(2000) } as i64;
        let fts = fts_query(&q.text);
        let mut sql = String::from(
            "SELECT b.id, b.cmd, b.cwd, b.exit_code, b.started_ms, b.ended_ms, b.starred, b.note, b.template, ",
        );
        sql.push_str(if fts.is_some() {
            "snippet(blocks_fts, -1, char(1), char(2), '…', 24) FROM blocks_fts JOIN blocks b ON b.id = blocks_fts.rowid WHERE blocks_fts MATCH ?1"
        } else {
            "substr(b.output, 1, 200) FROM blocks b WHERE ?1 IS NULL"
        });
        if q.failed_only {
            sql.push_str(" AND b.exit_code IS NOT NULL AND b.exit_code != 0");
        }
        if q.starred_only {
            sql.push_str(" AND b.starred = 1");
        }
        sql.push_str(" AND (?2 IS NULL OR b.cwd = ?2 OR b.cwd LIKE ?3 ESCAPE '\\')");
        sql.push_str(" AND (?4 IS NULL OR b.started_ms >= ?4) AND (?5 IS NULL OR b.started_ms <= ?5)");
        sql.push_str(" ORDER BY b.started_ms DESC LIMIT ?6");
        let like = q.cwd_prefix.as_ref().map(|p| {
            let esc = p.trim_end_matches('/').replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_");
            format!("{esc}/%")
        });
        let mut stmt = self.conn.prepare_cached(&sql)?;
        let rows = stmt.query_map(
            params![fts, q.cwd_prefix, like, q.since_ms.map(|v| v as i64), q.until_ms.map(|v| v as i64), limit],
            |r| {
                let started: i64 = r.get(4)?;
                let ended: i64 = r.get(5)?;
                Ok(HistoryHit {
                    id: r.get(0)?,
                    cmd: r.get(1)?,
                    cwd: r.get(2)?,
                    exit_code: r.get(3)?,
                    started_ms: started as u64,
                    duration_ms: (ended - started).max(0) as u64,
                    starred: r.get(6)?,
                    note: r.get(7)?,
                    template: r.get(8)?,
                    snippet: r.get(9)?,
                })
            },
        )?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    pub fn get(&self, id: i64) -> Result<Option<Entry>> {
        Ok(self
            .conn
            .query_row(
                "SELECT id, cmd, cwd, exit_code, started_ms, ended_ms, starred, note, output, truncated, template FROM blocks WHERE id = ?1",
                [id],
                |r| {
                    let started: i64 = r.get(4)?;
                    let ended: i64 = r.get(5)?;
                    Ok(Entry {
                        hit: HistoryHit {
                            id: r.get(0)?,
                            cmd: r.get(1)?,
                            cwd: r.get(2)?,
                            exit_code: r.get(3)?,
                            started_ms: started as u64,
                            duration_ms: (ended - started).max(0) as u64,
                            starred: r.get(6)?,
                            note: r.get(7)?,
                            template: r.get(10)?,
                            snippet: String::new(),
                        },
                        output: r.get(8)?,
                        truncated: r.get(9)?,
                    })
                },
            )
            .optional()?)
    }

    pub fn set_starred(&self, id: i64, starred: bool) -> Result<()> {
        self.conn.execute("UPDATE blocks SET starred = ?2 WHERE id = ?1", params![id, starred])?;
        Ok(())
    }

    pub fn set_note(&self, id: i64, note: Option<&str>) -> Result<()> {
        self.conn.execute("UPDATE blocks SET note = ?2 WHERE id = ?1", params![id, note])?;
        Ok(())
    }

    pub fn set_template(&self, id: i64, template: Option<&str>) -> Result<()> {
        self.conn.execute("UPDATE blocks SET template = ?2 WHERE id = ?1", params![id, template])?;
        Ok(())
    }

    pub fn forget(&self, id: i64) -> Result<()> {
        self.conn.execute("DELETE FROM blocks WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn size_bytes(&self) -> Result<u64> {
        let pages: i64 = self.conn.pragma_query_value(None, "page_count", |r| r.get(0))?;
        let size: i64 = self.conn.pragma_query_value(None, "page_size", |r| r.get(0))?;
        let free: i64 = self.conn.pragma_query_value(None, "freelist_count", |r| r.get(0))?;
        Ok(((pages - free).max(0) * size) as u64)
    }

    pub fn enforce_retention(&mut self, max_bytes: u64, max_age_days: Option<u32>, now_ms: u64) -> Result<usize> {
        let mut removed = 0;
        if let Some(days) = max_age_days {
            let cutoff = now_ms.saturating_sub(days as u64 * 86_400_000) as i64;
            removed += self.conn.execute("DELETE FROM blocks WHERE starred = 0 AND started_ms < ?1", [cutoff])?;
        }
        while self.size_bytes()? > max_bytes {
            let n = self.conn.execute(
                "DELETE FROM blocks WHERE id IN (SELECT id FROM blocks WHERE starred = 0 ORDER BY started_ms LIMIT 500)",
                [],
            )?;
            if n == 0 {
                break;
            }
            removed += n;
            self.conn.execute("INSERT INTO blocks_fts(blocks_fts) VALUES ('optimize')", [])?;
            self.conn.execute_batch("PRAGMA incremental_vacuum;")?;
        }
        if removed > 0 {
            self.conn.execute_batch("PRAGMA incremental_vacuum;")?;
        }
        Ok(removed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(cmd: &str, output: &str, exit: i32, started: u64, cwd: &str) -> NewBlock {
        NewBlock {
            host: "local".into(),
            cwd: Some(cwd.into()),
            cmd: cmd.into(),
            exit_code: Some(exit),
            started_ms: started,
            ended_ms: started + 1500,
            output: output.into(),
        }
    }

    fn q(text: &str) -> HistoryQuery {
        HistoryQuery { text: text.into(), ..Default::default() }
    }

    fn sample() -> History {
        let mut h = History::open_in_memory().unwrap();
        h.insert(&[
            block("cargo build", "Compiling zhell\nerror[E0308]: mismatched types", 101, 1_000, "/home/z/zhell"),
            block("ls -la", "total 0\ndrwx certificate.pem", 0, 2_000, "/home/z"),
            block("curl https://x", "SSL certificate problem: unable to get local issuer", 60, 3_000, "/home/z/api"),
            block("export TOKEN=abcdef123456", "", 0, 4_000, "/home/z"),
        ])
        .unwrap();
        h
    }

    #[test]
    fn full_text_search_with_snippets_newest_first() {
        let h = sample();
        let hits = h.search(&q("certificate")).unwrap();
        assert_eq!(hits.iter().map(|h| h.cmd.as_str()).collect::<Vec<_>>(), vec!["curl https://x", "ls -la"]);
        assert!(hits[0].snippet.contains("\u{1}certificate\u{2}"), "{:?}", hits[0].snippet);
        assert_eq!(hits[0].duration_ms, 1500);

        assert_eq!(h.search(&q("mismatch")).unwrap().len(), 1);
        assert_eq!(h.search(&q("certificate issuer")).unwrap().len(), 1);
        assert!(h.search(&q("nothing-like-this")).unwrap().is_empty());
    }

    #[test]
    fn filters() {
        let h = sample();
        let failed = h.search(&HistoryQuery { failed_only: true, ..q("") }).unwrap();
        assert_eq!(failed.len(), 2);
        let under = h.search(&HistoryQuery { cwd_prefix: Some("/home/z/zhell".into()), ..q("") }).unwrap();
        assert_eq!(under.len(), 1);

        assert!(h.search(&HistoryQuery { cwd_prefix: Some("/home/z/zhe".into()), ..q("") }).unwrap().is_empty());
        let recent = h.search(&HistoryQuery { since_ms: Some(2_500), ..q("") }).unwrap();
        assert_eq!(recent.len(), 2);
    }

    #[test]
    fn fts_syntax_in_queries_is_harmless() {
        let h = sample();
        for text in ["\"", "AND", "-x", "cwd:foo", "NEAR(a b)", "*", "a OR"] {
            h.search(&q(text)).unwrap_or_else(|e| panic!("{text:?}: {e}"));
        }
    }

    #[test]
    fn secrets_never_reach_the_database() {
        let h = sample();
        assert!(h.search(&q("abcdef123456")).unwrap().is_empty());
        let hit = &h.search(&q("export")).unwrap()[0];
        assert_eq!(hit.cmd, "export TOKEN=‹redacted:secret›");
        let raw: i64 = h.conn.query_row("SELECT count(*) FROM blocks WHERE cmd LIKE '%abcdef123456%' OR output LIKE '%abcdef123456%'", [], |r| r.get(0)).unwrap();
        assert_eq!(raw, 0);
    }

    #[test]
    fn get_star_note_forget() {
        let h = sample();
        let id = h.search(&q("cargo")).unwrap()[0].id;
        h.set_starred(id, true).unwrap();
        h.set_note(id, Some("the type error fix")).unwrap();
        let e = h.get(id).unwrap().unwrap();
        assert!(e.hit.starred && e.output.contains("E0308"));
        assert_eq!(e.hit.note.as_deref(), Some("the type error fix"));
        assert_eq!(h.search(&HistoryQuery { starred_only: true, ..q("") }).unwrap().len(), 1);
        h.set_template(id, Some("cargo build -p {{crate}}")).unwrap();
        assert_eq!(h.get(id).unwrap().unwrap().hit.template.as_deref(), Some("cargo build -p {{crate}}"));
        h.forget(id).unwrap();
        assert!(h.get(id).unwrap().is_none());
        assert!(h.search(&q("mismatched")).unwrap().is_empty(), "forgotten text must leave the index");
    }

    #[test]
    fn encryption_and_conversion() {
        let dir = std::env::temp_dir().join(format!("zhell-hist-enc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("history.db");
        let key = "ab".repeat(32);
        {
            let mut h = History::open(&path).unwrap();
            h.insert(&[block("plain entry", "x", 0, 1, "/")]).unwrap();
        }
        {
            let h = History::open_with_key(&path, Some(&key)).unwrap();
            assert_eq!(h.search(&q("plain")).unwrap().len(), 1);
        }

        assert!(matches!(History::open(&path), Err(Error::WrongKey)));
        let raw = std::fs::read(&path).unwrap();
        assert!(!raw.windows(11).any(|w| w == b"plain entry"), "plaintext leaked into the file");

        assert!(History::open_with_key(&path, Some(&"cd".repeat(32))).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn huge_output_is_capped() {
        let (s, t) = cap_output(&"é".repeat(MAX_OUTPUT_BYTES));
        assert!(t && s.len() < MAX_OUTPUT_BYTES + 100 && s.contains("bytes omitted"));
    }

    #[test]
    fn retention_by_age_keeps_starred() {
        let mut h = sample();
        let star = h.search(&q("cargo")).unwrap()[0].id;
        h.set_starred(star, true).unwrap();
        let removed = h.enforce_retention(u64::MAX, Some(1), 86_400_000 + 2_500).unwrap();
        assert_eq!(removed, 1, "only the unstarred ls at t=2000 is older than a day");
        assert!(h.get(star).unwrap().is_some());
    }
}
