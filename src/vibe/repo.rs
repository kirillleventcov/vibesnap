use crate::error::{AppError, Result};
use rusqlite::Connection;
use std::path::{Path, PathBuf};

use super::constants::REPO_DIRNAME;
use super::db::{db_connect, ensure_schema};

pub struct Repo {
    /// Canonical path of the directory that contains `.vibe`.
    pub root: PathBuf,
    pub conn: Connection,
}

#[derive(Debug, Clone)]
pub struct Head {
    pub track: String,
    pub checkpoint: Option<String>,
}

impl Repo {
    pub fn open() -> Result<Repo> {
        Repo::open_at(find_repo_root(None)?)
    }

    pub fn open_at(root: PathBuf) -> Result<Repo> {
        let mut conn = db_connect(&root)?;
        ensure_schema(&root, &mut conn)?;
        Ok(Repo { root, conn })
    }

    pub fn head(&self) -> Result<Head> {
        let head = self.conn.query_row(
            "SELECT track, checkpoint FROM head WHERE id = 1",
            [],
            |row| {
                Ok(Head {
                    track: row.get(0)?,
                    checkpoint: row.get(1)?,
                })
            },
        )?;
        Ok(head)
    }
}

pub fn set_head(conn: &Connection, track: &str, checkpoint: Option<&str>) -> Result<()> {
    conn.execute(
        "INSERT INTO head(id, track, checkpoint) VALUES (1, ?1, ?2)
         ON CONFLICT(id) DO UPDATE SET track = excluded.track, checkpoint = excluded.checkpoint",
        rusqlite::params![track, checkpoint],
    )?;
    Ok(())
}

pub fn find_repo_root(start: Option<PathBuf>) -> Result<PathBuf> {
    let current_dir: PathBuf = match start {
        Some(dir) => dir,
        None => std::env::current_dir()?,
    };
    let current_dir = current_dir.canonicalize().unwrap_or(current_dir);
    let mut current: &Path = current_dir.as_path();

    loop {
        if current.join(REPO_DIRNAME).is_dir() {
            return Ok(current.to_path_buf());
        }
        match current.parent() {
            Some(parent) => current = parent,
            None => return Err(AppError::NotInRepo),
        }
    }
}
