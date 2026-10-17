//! Migration from the v1 layout (flat SHA-256 objects, JSON manifests and a
//! `meta.db`). The old store is kept at `.vibe/v1`, and every v1 checkpoint is
//! imported as a label `v1-<ID>`. A v1 manifest does not reliably say whether it
//! covered the whole tree, so unless it is marked complete the label is partial:
//! restoring it writes files but never deletes any.

use super::odb::{self, Entry, LABELS_PREFIX, MODE_FILE, TreeEdit};
use super::{GitInfo, STORE_DIR, Store, add_to_git_exclude, create_layout, lock};
use crate::error::Result;
use crate::fsutil::tmp_path;
use crate::paths::is_safe_path;
use rusqlite::{Connection, OpenFlags};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

pub fn migrate(root: &Path) -> Result<()> {
    let vibe = root.join(STORE_DIR);
    let old_lock = lock::lock(&vibe)?;
    if vibe.join("version").exists() {
        return Ok(()); // another process migrated while we waited
    }
    let aside = tmp_path(root, ".vibe-v1-");
    fs::rename(&vibe, &aside)?;
    drop(old_lock);

    let git = GitInfo::detect(root);
    let tmp = tmp_path(root, ".vibe-init-");
    create_layout(&tmp, git.as_ref())?;
    fs::rename(&aside, tmp.join("v1"))?;
    fs::rename(&tmp, &vibe)?;
    if let Some(git) = &git {
        add_to_git_exclude(git)?;
    }

    let mut store = Store::open(root)?;
    let imported = import(&store)?;
    eprintln!(
        "vibesnap: upgraded the store to v2; the old one is in .vibe/v1. Imported {imported} checkpoints as labels v1-<ID>."
    );
    let lock = lock::lock(&store.vibe)?;
    crate::wc::scan::reconcile(
        &mut store,
        &lock,
        &super::ledger::EventMeta::cause("baseline", "user"),
    )?;
    Ok(())
}

fn import(store: &Store) -> Result<usize> {
    let old = store.vibe.join("v1");
    let conn = Connection::open_with_flags(old.join("meta.db"), OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let has_complete = conn
        .prepare("SELECT complete FROM checkpoints LIMIT 0")
        .is_ok();
    let query = if has_complete {
        "SELECT id, track, timestamp, note, complete FROM checkpoints ORDER BY timestamp, rowid"
    } else {
        "SELECT id, track, timestamp, note, NULL FROM checkpoints ORDER BY timestamp, rowid"
    };
    let mut stmt = conn.prepare(query)?;
    type Row = (String, Option<String>, i64, Option<String>, Option<bool>);
    let rows: Vec<Row> = stmt
        .query_map([], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        })?
        .collect::<rusqlite::Result<_>>()?;

    let repo = store.repo()?;
    let mut imported = 0;
    for (id, track, timestamp, note, complete) in rows {
        let manifest = match fs::read_to_string(old.join("snapshots").join(format!("{id}.json"))) {
            Ok(m) => m,
            Err(_) => continue,
        };
        #[derive(serde::Deserialize)]
        struct Manifest {
            files: BTreeMap<String, String>,
        }
        let Ok(manifest) = serde_json::from_str::<Manifest>(&manifest) else {
            continue;
        };
        let mut edits = Vec::new();
        for (path, hash) in manifest.files {
            if !is_safe_path(&path) {
                continue;
            }
            let Ok(content) = fs::read(old.join("objects").join(&hash)) else {
                continue;
            };
            let oid = odb::write_blob(repo, &content)?;
            edits.push(TreeEdit::Upsert(
                path,
                Entry {
                    oid,
                    mode: MODE_FILE,
                },
            ));
        }
        let tree = odb::edit_tree(repo, odb::empty_tree(), &edits)?;
        let partial = complete != Some(true);
        let message = format!(
            "import v1 checkpoint {id}: {}\n\nVibesnap-Import: v1\nVibesnap-Partial: {partial}\nVibesnap-Track: {}\n",
            note.unwrap_or_default().replace('\n', " "),
            track.unwrap_or_default()
        );
        let commit = odb::write_commit(repo, tree, &[], &message, timestamp)?;
        odb::force_ref(repo, &format!("{LABELS_PREFIX}v1-{id}"), commit)?;
        imported += 1;
    }
    Ok(imported)
}
