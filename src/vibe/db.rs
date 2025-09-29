use crate::error::{AppError, Result};
use rusqlite::Connection;
use std::path::Path;

use super::constants::{DB_FILENAME, REPO_DIRNAME};

pub fn db_connect(root: &Path) -> Result<Connection> {
    let db_path = root.join(REPO_DIRNAME).join(DB_FILENAME);
    Connection::open(db_path).map_err(AppError::DbError)
}

pub fn ensure_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
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
    )
    .map_err(AppError::DbError)
}
