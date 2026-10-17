//! The SQLite index over the ledger. Git holds the truth; everything here can be
//! rebuilt from the ledger's commit trailers with `vs reindex`, except the stat
//! cache (rebuilt by the next scan) and subagent summaries.

use crate::error::Result;
use rusqlite::{Connection, OptionalExtension, Row};
use serde::Serialize;
use std::path::Path;
use std::time::Duration;

const SCHEMA_VERSION: i32 = 1;

pub fn connect(vibe: &Path) -> Result<Connection> {
    let conn = Connection::open(vibe.join("index.db"))?;
    conn.busy_timeout(Duration::from_secs(5))?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "foreign_keys", "OFF")?;
    conn.pragma_update(None, "mmap_size", 256 * 1024 * 1024)?;
    // Checkpoint little and often, and shrink the WAL afterwards, so no single
    // hook pays for a large checkpoint and the WAL stays small on disk.
    conn.pragma_update(None, "wal_autocheckpoint", 256)?;
    conn.pragma_update(None, "journal_size_limit", 1024 * 1024)?;
    // Every hook is its own process. Without this, each one would checkpoint
    // and delete the WAL on exit (with fsyncs) and recreate it on the next
    // open. The WAL is still checkpointed automatically as it grows.
    conn.set_db_config(
        rusqlite::config::DbConfig::SQLITE_DBCONFIG_NO_CKPT_ON_CLOSE,
        true,
    )?;
    let version: i32 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if version < SCHEMA_VERSION {
        conn.pragma_update_and_check(None, "journal_mode", "WAL", |r| r.get::<_, String>(0))?;
        conn.execute_batch(SCHEMA)?;
        conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    }
    Ok(conn)
}

const SCHEMA: &str = "
BEGIN IMMEDIATE;
CREATE TABLE IF NOT EXISTS events (
  seq INTEGER PRIMARY KEY AUTOINCREMENT,
  commit_oid TEXT NOT NULL,
  ts_ns INTEGER NOT NULL,
  before_tree TEXT,
  after_tree TEXT NOT NULL,
  cause TEXT NOT NULL,
  actor TEXT NOT NULL,
  session_id TEXT,
  turn INTEGER,
  prompt_id TEXT,
  agent_id TEXT,
  agent_type TEXT,
  tool TEXT,
  tool_use_id TEXT,
  lane TEXT NOT NULL DEFAULT 'main',
  note TEXT,
  ambiguous TEXT
);
CREATE INDEX IF NOT EXISTS events_ts ON events (ts_ns);
CREATE INDEX IF NOT EXISTS events_turn ON events (turn);
CREATE TABLE IF NOT EXISTS event_paths (
  seq INTEGER NOT NULL,
  path TEXT NOT NULL,
  before_oid TEXT,
  after_oid TEXT,
  before_mode INTEGER,
  after_mode INTEGER,
  added INTEGER,
  removed INTEGER,
  PRIMARY KEY (path, seq)
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS event_paths_seq ON event_paths (seq);
CREATE TABLE IF NOT EXISTS spans (
  id TEXT PRIMARY KEY,
  kind TEXT NOT NULL,
  parent TEXT,
  session_id TEXT,
  number INTEGER,
  agent_type TEXT,
  start_seq INTEGER NOT NULL,
  end_seq INTEGER,
  start_ns INTEGER,
  end_ns INTEGER,
  summary TEXT
);
CREATE INDEX IF NOT EXISTS spans_kind ON spans (kind, number);
CREATE TABLE IF NOT EXISTS commit_offers (
  turn_end_seq INTEGER PRIMARY KEY,
  offered_ns INTEGER,
  committed_oid TEXT
);
CREATE TABLE IF NOT EXISTS stat_cache (
  lane TEXT NOT NULL,
  path TEXT NOT NULL,
  mtime_ns INTEGER NOT NULL,
  ctime_ns INTEGER NOT NULL,
  size INTEGER NOT NULL,
  ino INTEGER NOT NULL,
  mode INTEGER NOT NULL,
  oid TEXT NOT NULL,
  racy INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (lane, path)
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS intents (
  tool_use_id TEXT PRIMARY KEY,
  path TEXT NOT NULL,
  opened_ns INTEGER NOT NULL,
  pre_oid TEXT,
  pre_mode INTEGER,
  agent_id TEXT
);
CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT);
COMMIT;
";

#[derive(Debug, Clone, Serialize, Default)]
pub struct Event {
    pub seq: i64,
    pub commit_oid: String,
    pub ts_ns: i64,
    pub before_tree: Option<String>,
    pub after_tree: String,
    pub cause: String,
    pub actor: String,
    pub session_id: Option<String>,
    pub turn: Option<i64>,
    pub prompt_id: Option<String>,
    pub agent_id: Option<String>,
    pub agent_type: Option<String>,
    pub tool: Option<String>,
    pub tool_use_id: Option<String>,
    pub lane: String,
    pub note: Option<String>,
    pub ambiguous: Option<String>,
}

impl Event {
    /// "main" for the main agent, the subagent type for subagents, otherwise the actor.
    pub fn who(&self) -> String {
        if self.actor == "agent" {
            match &self.agent_type {
                Some(t) if !t.is_empty() => t.clone(),
                _ => "main".into(),
            }
        } else {
            self.actor.clone()
        }
    }
}

pub const EVENT_COLUMNS: &str = "seq, commit_oid, ts_ns, before_tree, after_tree, cause, actor, session_id, turn, prompt_id, agent_id, agent_type, tool, tool_use_id, lane, note, ambiguous";

pub fn event_from_row(row: &Row) -> rusqlite::Result<Event> {
    Ok(Event {
        seq: row.get(0)?,
        commit_oid: row.get(1)?,
        ts_ns: row.get(2)?,
        before_tree: row.get(3)?,
        after_tree: row.get(4)?,
        cause: row.get(5)?,
        actor: row.get(6)?,
        session_id: row.get(7)?,
        turn: row.get(8)?,
        prompt_id: row.get(9)?,
        agent_id: row.get(10)?,
        agent_type: row.get(11)?,
        tool: row.get(12)?,
        tool_use_id: row.get(13)?,
        lane: row.get(14)?,
        note: row.get(15)?,
        ambiguous: row.get(16)?,
    })
}

pub fn last_event(conn: &Connection) -> Result<Option<Event>> {
    Ok(conn
        .query_row(
            &format!("SELECT {EVENT_COLUMNS} FROM events ORDER BY seq DESC LIMIT 1"),
            [],
            event_from_row,
        )
        .optional()?)
}

pub fn event(conn: &Connection, seq: i64) -> Result<Option<Event>> {
    Ok(conn
        .query_row(
            &format!("SELECT {EVENT_COLUMNS} FROM events WHERE seq = ?1"),
            [seq],
            event_from_row,
        )
        .optional()?)
}

/// Events with `from < seq <= to`, oldest first.
pub fn events_between(conn: &Connection, from: i64, to: i64) -> Result<Vec<Event>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {EVENT_COLUMNS} FROM events WHERE seq > ?1 AND seq <= ?2 ORDER BY seq"
    ))?;
    let rows = stmt.query_map([from, to], event_from_row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub fn last_seq(conn: &Connection) -> Result<i64> {
    Ok(conn.query_row("SELECT COALESCE(MAX(seq), 0) FROM events", [], |r| r.get(0))?)
}

/// The next sequence number; never reuses one, even after pruning.
pub fn next_seq(conn: &Connection) -> Result<i64> {
    let from_sequence: Option<i64> = conn
        .query_row(
            "SELECT seq FROM sqlite_sequence WHERE name = 'events'",
            [],
            |r| r.get(0),
        )
        .optional()?;
    Ok(from_sequence.unwrap_or(0).max(last_seq(conn)?) + 1)
}

#[derive(Debug, Clone, Serialize)]
pub struct PathRow {
    pub seq: i64,
    pub path: String,
    pub before_oid: Option<String>,
    pub after_oid: Option<String>,
    pub before_mode: Option<u32>,
    pub after_mode: Option<u32>,
    pub added: Option<i64>,
    pub removed: Option<i64>,
}

fn path_row(row: &Row) -> rusqlite::Result<PathRow> {
    Ok(PathRow {
        seq: row.get(0)?,
        path: row.get(1)?,
        before_oid: row.get(2)?,
        after_oid: row.get(3)?,
        before_mode: row.get(4)?,
        after_mode: row.get(5)?,
        added: row.get(6)?,
        removed: row.get(7)?,
    })
}

const PATH_COLUMNS: &str =
    "seq, path, before_oid, after_oid, before_mode, after_mode, added, removed";

pub fn event_paths(conn: &Connection, seq: i64) -> Result<Vec<PathRow>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {PATH_COLUMNS} FROM event_paths WHERE seq = ?1 ORDER BY path"
    ))?;
    let rows = stmt.query_map([seq], path_row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Every change to paths under `prefix` (or exactly `prefix`) in `from < seq <= to`, oldest first.
pub fn path_history(conn: &Connection, prefix: &str, from: i64, to: i64) -> Result<Vec<PathRow>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {PATH_COLUMNS} FROM event_paths
         WHERE (?1 = '' OR path = ?1 OR (path >= ?1 || '/' AND path < ?1 || '0'))
           AND seq > ?2 AND seq <= ?3
         ORDER BY seq, path"
    ))?;
    let rows = stmt.query_map(rusqlite::params![prefix, from, to], path_row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

#[derive(Debug, Clone, Serialize)]
pub struct Span {
    pub id: String,
    pub kind: String,
    pub parent: Option<String>,
    pub session_id: Option<String>,
    pub number: Option<i64>,
    pub agent_type: Option<String>,
    pub start_seq: i64,
    pub end_seq: Option<i64>,
    pub start_ns: Option<i64>,
    pub end_ns: Option<i64>,
    pub summary: Option<String>,
}

pub const SPAN_COLUMNS: &str = "id, kind, parent, session_id, number, agent_type, start_seq, end_seq, start_ns, end_ns, summary";

pub fn span_from_row(row: &Row) -> rusqlite::Result<Span> {
    Ok(Span {
        id: row.get(0)?,
        kind: row.get(1)?,
        parent: row.get(2)?,
        session_id: row.get(3)?,
        number: row.get(4)?,
        agent_type: row.get(5)?,
        start_seq: row.get(6)?,
        end_seq: row.get(7)?,
        start_ns: row.get(8)?,
        end_ns: row.get(9)?,
        summary: row.get(10)?,
    })
}

pub fn span(conn: &Connection, id: &str) -> Result<Option<Span>> {
    Ok(conn
        .query_row(
            &format!("SELECT {SPAN_COLUMNS} FROM spans WHERE id = ?1"),
            [id],
            span_from_row,
        )
        .optional()?)
}

/// Spans of `kind`, newest first.
pub fn spans(conn: &Connection, kind: &str) -> Result<Vec<Span>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {SPAN_COLUMNS} FROM spans WHERE kind = ?1 ORDER BY start_seq DESC, start_ns DESC"
    ))?;
    let rows = stmt.query_map([kind], span_from_row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub fn get_meta(conn: &Connection, key: &str) -> Result<Option<String>> {
    Ok(conn
        .query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0))
        .optional()?)
}

pub fn set_meta(conn: &Connection, key: &str, value: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO meta(key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [key, value],
    )?;
    Ok(())
}
