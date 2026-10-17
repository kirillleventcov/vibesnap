use crate::error::{AppError, Result};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::hash::{BuildHasher, RandomState};

use super::lock::RepoLock;
use super::objects::ObjectStore;
use super::paths::in_scope;
use super::repo::{Repo, set_head};
use super::snapshot::{
    SnapshotManifest, load_snapshot_manifest, overlay, remove_manifest, scan, write_new_manifest,
};

#[derive(Debug, Clone, Serialize)]
pub struct Checkpoint {
    pub id: String,
    pub track: String,
    pub parent: Option<String>,
    pub timestamp: i64,
    pub note: Option<String>,
    /// Whether the manifest describes the whole tree. `None` for checkpoints made
    /// by versions that did not record it.
    pub complete: Option<bool>,
}

const COLUMNS: &str = "id, track, parent, timestamp, note, complete";

fn from_row(row: &rusqlite::Row) -> rusqlite::Result<Checkpoint> {
    Ok(Checkpoint {
        id: row.get(0)?,
        track: row.get(1)?,
        parent: row.get(2)?,
        timestamp: row.get(3)?,
        note: row.get(4)?,
        complete: row.get(5)?,
    })
}

pub fn find_checkpoint(conn: &Connection, id: &str) -> Result<Checkpoint> {
    conn.query_row(
        &format!("SELECT {COLUMNS} FROM checkpoints WHERE id = ?1 COLLATE NOCASE"),
        [id],
        from_row,
    )
    .optional()?
    .ok_or_else(|| AppError::CheckpointNotFound(id.to_string()))
}

/// Checkpoints in creation order (oldest first).
pub fn list_checkpoints(conn: &Connection, track: Option<&str>) -> Result<Vec<Checkpoint>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM checkpoints WHERE ?1 IS NULL OR track = ?1 ORDER BY timestamp, rowid"
    ))?;
    let rows = stmt.query_map([track], from_row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub fn generate_id(track: &str) -> String {
    let timestamp: i64 = chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default();
    // RandomState is seeded per process and per call, so parallel processes that
    // read the same clock still get different IDs.
    let random = RandomState::new().hash_one((track, timestamp, std::process::id()));
    let mut hasher = Sha256::new();
    hasher.update(format!("{track}{timestamp}{random}").as_bytes());
    hex::encode(&hasher.finalize()[..4]).to_uppercase()
}

pub struct NewCheckpoint<'a> {
    pub track: &'a str,
    pub parent: Option<&'a str>,
    pub note: &'a str,
    pub complete: bool,
}

/// Stores a checkpoint and moves the track head and HEAD to it. The manifest is
/// written first (exclusively, so a colliding ID is skipped rather than
/// overwritten), then the row, track head and HEAD change in one transaction.
/// A crash in between leaves at worst an orphaned manifest file.
pub fn create_checkpoint(
    repo: &mut Repo,
    lock: &RepoLock,
    new: &NewCheckpoint,
    manifest: &SnapshotManifest,
) -> Result<String> {
    let ids = std::iter::repeat_with(|| generate_id(new.track)).take(32);
    create_checkpoint_with_ids(repo, lock, new, manifest, ids)
}

fn create_checkpoint_with_ids(
    repo: &mut Repo,
    _lock: &RepoLock,
    new: &NewCheckpoint,
    manifest: &SnapshotManifest,
    ids: impl Iterator<Item = String>,
) -> Result<String> {
    for id in ids {
        let taken: bool = repo.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM checkpoints WHERE id = ?1 COLLATE NOCASE)",
            [&id],
            |row| row.get(0),
        )?;
        if taken || !write_new_manifest(&repo.root, &id, manifest)? {
            continue;
        }
        if let Err(e) = insert_checkpoint(&mut repo.conn, &id, new) {
            remove_manifest(&repo.root, &id);
            return Err(e);
        }
        return Ok(id);
    }
    Err(AppError::Generic(
        "Could not allocate a unique checkpoint ID".into(),
    ))
}

fn insert_checkpoint(conn: &mut Connection, id: &str, new: &NewCheckpoint) -> Result<()> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    tx.execute(
        "INSERT INTO checkpoints(id, track, parent, timestamp, note, complete) VALUES (?, ?, ?, ?, ?, ?)",
        rusqlite::params![
            id,
            new.track,
            new.parent,
            chrono::Utc::now().timestamp(),
            new.note,
            new.complete
        ],
    )?;
    tx.execute(
        "INSERT INTO tracks(name, head) VALUES (?1, ?2)
         ON CONFLICT(name) DO UPDATE SET head = excluded.head",
        rusqlite::params![new.track, id],
    )?;
    set_head(&tx, new.track, Some(id))?;
    tx.commit()?;
    Ok(())
}

/// The result of recording the working tree.
pub struct Recorded {
    pub track: String,
    /// HEAD after recording: the new checkpoint, or the unchanged parent.
    pub id: Option<String>,
    pub parent: Option<String>,
    /// False when nothing changed since HEAD, so no checkpoint was created.
    pub created: bool,
    pub complete: bool,
    pub manifest: SnapshotManifest,
    /// What was read from disk within the scope.
    pub scanned: SnapshotManifest,
}

/// Records the working tree as a checkpoint on the current track. With an empty
/// scope the whole tree is read. Otherwise only the scope is read and laid over
/// HEAD's state, so the checkpoint is still a whole-tree state if HEAD was one.
/// Nothing is created when the result equals HEAD.
pub fn record_state(
    repo: &mut Repo,
    lock: &RepoLock,
    scope: &[String],
    note: &str,
) -> Result<Recorded> {
    let head = repo.head()?;
    let parent = match &head.checkpoint {
        Some(id) => Some(find_checkpoint(&repo.conn, id)?),
        None => None,
    };
    let parent_manifest = match &parent {
        Some(p) => Some(load_snapshot_manifest(&repo.root, &p.id)?),
        None => None,
    };

    let store = ObjectStore::open(&repo.root)?;
    let whole_tree = scope.iter().all(|p| p.is_empty());
    let scanned = scan(&repo.root, scope, &store)?;
    let (manifest, complete) = if whole_tree {
        (scanned.clone(), true)
    } else {
        let complete = parent.as_ref().and_then(|p| p.complete) == Some(true);
        (overlay(parent_manifest.as_ref(), scope, &scanned), complete)
    };

    let unchanged = parent
        .as_ref()
        .is_some_and(|p| p.complete == Some(complete))
        && parent_manifest.as_ref() == Some(&manifest);
    let parent_id = parent.map(|p| p.id);
    if unchanged {
        return Ok(Recorded {
            track: head.track,
            id: parent_id.clone(),
            parent: parent_id,
            created: false,
            complete,
            manifest,
            scanned,
        });
    }

    let new = NewCheckpoint {
        track: &head.track,
        parent: parent_id.as_deref(),
        note,
        complete,
    };
    let id = create_checkpoint(repo, lock, &new, &manifest)?;
    Ok(Recorded {
        track: head.track,
        id: Some(id),
        parent: parent_id,
        created: true,
        complete,
        manifest,
        scanned,
    })
}

/// True if `manifest` has an entry at `path` or below it.
pub fn manifest_covers(manifest: &SnapshotManifest, path: &str) -> bool {
    let scope = [path.to_string()];
    manifest.files.keys().any(|p| in_scope(p, &scope))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vibe::constants::REPO_DIRNAME;
    use crate::vibe::lock::lock_repo;

    #[test]
    fn colliding_id_is_skipped_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(REPO_DIRNAME)).unwrap();
        let mut repo = Repo::open_at(dir.path().to_path_buf()).unwrap();
        let lock = lock_repo(&repo.root).unwrap();
        let new = NewCheckpoint {
            track: "main",
            parent: None,
            note: "",
            complete: true,
        };
        let mut first = SnapshotManifest::default();
        first.files.insert("a".into(), "1".into());
        let mut second = SnapshotManifest::default();
        second.files.insert("b".into(), "2".into());

        let ids = ["AAAA0000".to_string()].into_iter();
        let id1 = create_checkpoint_with_ids(&mut repo, &lock, &new, &first, ids).unwrap();
        let ids = ["AAAA0000".to_string(), "BBBB0000".to_string()].into_iter();
        let id2 = create_checkpoint_with_ids(&mut repo, &lock, &new, &second, ids).unwrap();

        assert_eq!(id1, "AAAA0000");
        assert_eq!(id2, "BBBB0000");
        assert_eq!(
            load_snapshot_manifest(&repo.root, "AAAA0000").unwrap(),
            first
        );
        assert_eq!(
            load_snapshot_manifest(&repo.root, "BBBB0000").unwrap(),
            second
        );
        assert_eq!(list_checkpoints(&repo.conn, None).unwrap().len(), 2);
    }
}
