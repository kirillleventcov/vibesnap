use crate::error::{AppError, Result};
use rusqlite::{Connection, TransactionBehavior};
use std::fs;
use std::path::Path;
use std::time::Duration;

use super::constants::{DB_FILENAME, DEFAULT_TRACK, HEAD_FILENAME, REPO_DIRNAME};
use super::lock::lock_repo;

/// Bumped whenever `migrate` learns a new step.
/// 1: tracks + checkpoints (the original layout, which never set user_version).
/// 2: checkpoints.complete, HEAD moved into the `head` table, WAL journal.
const SCHEMA_VERSION: i32 = 2;

pub fn db_connect(root: &Path) -> Result<Connection> {
    let db_path = root.join(REPO_DIRNAME).join(DB_FILENAME);
    let conn = Connection::open(db_path).map_err(AppError::Db)?;
    conn.busy_timeout(Duration::from_secs(5))?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    Ok(conn)
}

/// Creates or upgrades the schema. Cheap when already current; otherwise takes the
/// writer lock so two processes never migrate at once.
pub fn ensure_schema(root: &Path, conn: &mut Connection) -> Result<()> {
    if schema_version(conn)? >= SCHEMA_VERSION {
        return Ok(());
    }
    let _lock = lock_repo(root)?;
    // journal_mode cannot change inside a transaction.
    conn.pragma_update_and_check(None, "journal_mode", "WAL", |row| row.get::<_, String>(0))?;

    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let version = schema_version(&tx)?;
    if version >= SCHEMA_VERSION {
        return Ok(());
    }
    if version < 1 {
        tx.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS tracks (
                name TEXT PRIMARY KEY,
                head TEXT
            );

            CREATE TABLE IF NOT EXISTS checkpoints (
                id TEXT PRIMARY KEY,
                track TEXT,
                parent TEXT,
                timestamp INTEGER,
                note TEXT,
                is_auto INTEGER DEFAULT 0
            );
            ",
        )?;
    }
    let mut imported_head_file = false;
    if version < 2 {
        // NULL means "written by a version that did not record it": a legacy snap may
        // have been partial, so restore never deletes files on its behalf.
        if !has_column(&tx, "checkpoints", "complete")? {
            tx.execute_batch("ALTER TABLE checkpoints ADD COLUMN complete INTEGER")?;
        }
        tx.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS head (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                track TEXT NOT NULL,
                checkpoint TEXT
            );
            ",
        )?;
        let (track, checkpoint) = read_legacy_head_file(root)?;
        tx.execute(
            "INSERT OR IGNORE INTO head(id, track, checkpoint) VALUES (1, ?1, ?2)",
            rusqlite::params![track, checkpoint],
        )?;
        imported_head_file = true;
    }
    tx.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    tx.commit()?;

    if imported_head_file {
        // The database is now the only source of truth for HEAD.
        let _ = fs::remove_file(root.join(REPO_DIRNAME).join(HEAD_FILENAME));
    }
    Ok(())
}

fn schema_version(conn: &Connection) -> Result<i32> {
    Ok(conn.pragma_query_value(None, "user_version", |row| row.get(0))?)
}

fn has_column(conn: &Connection, table: &str, column: &str) -> Result<bool> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let names = stmt.query_map([], |row| row.get::<_, String>(1))?;
    for name in names {
        if name? == column {
            return Ok(true);
        }
    }
    Ok(false)
}

fn read_legacy_head_file(root: &Path) -> Result<(String, Option<String>)> {
    let head_file = root.join(REPO_DIRNAME).join(HEAD_FILENAME);
    let Ok(content) = fs::read_to_string(head_file) else {
        return Ok((DEFAULT_TRACK.to_string(), None));
    };
    let parts: Vec<&str> = content.split_whitespace().collect();
    match parts.as_slice() {
        [track] => Ok((track.to_string(), None)),
        [track, checkpoint_id] => Ok((track.to_string(), Some(checkpoint_id.to_string()))),
        _ => Err(AppError::Generic("Invalid HEAD file format".into())),
    }
}
