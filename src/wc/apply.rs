//! Writing a recorded state into the working tree: only files that differ are
//! written (by temp file and rename, keeping mtimes of the rest), and files the
//! state does not have are deleted when the state describes the whole tree.

use super::filter::Oracle;
use crate::error::Result;
use crate::fsutil::{remove_empty_parents, symlink_atomic, write_atomic};
use crate::paths::{in_scope, is_safe_path};
use crate::store::Store;
use crate::store::odb::{self, Entry, MODE_EXEC, MODE_LINK};
use serde::Serialize;
use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

#[derive(Debug, Default, Serialize)]
pub struct Applied {
    pub written: Vec<String>,
    pub deleted: Vec<String>,
    pub unchanged: usize,
    pub skipped: Vec<Skipped>,
}

#[derive(Debug, Serialize)]
pub struct Skipped {
    pub path: String,
    pub reason: String,
}

/// Brings the files under `scope` from `current` to `target`. `current` must be
/// the recorded state of the working tree right now (the caller reconciles
/// first). Deletes only when `delete` is set.
pub fn apply(
    store: &Store,
    oracle: &mut Oracle,
    current: &BTreeMap<String, Entry>,
    target: &BTreeMap<String, Entry>,
    scope: &[String],
    delete: bool,
) -> Result<Applied> {
    let mut out = Applied::default();
    let root = &store.root;

    if delete {
        for path in current
            .keys()
            .filter(|p| in_scope(p, scope) && !target.contains_key(*p))
        {
            if !is_safe_path(path) {
                continue;
            }
            let abs = root.join(path);
            match fs::remove_file(&abs) {
                Ok(()) => {
                    remove_empty_parents(root, &abs);
                    out.deleted.push(path.clone());
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => out.skipped.push(Skipped {
                    path: path.clone(),
                    reason: format!("could not delete: {e}"),
                }),
            }
        }
    }

    for (path, entry) in target.iter().filter(|(p, _)| in_scope(p, scope)) {
        if current.get(path) == Some(entry) {
            out.unchanged += 1;
            continue;
        }
        if !is_safe_path(path) {
            out.skipped.push(Skipped {
                path: path.clone(),
                reason: "unsafe path".into(),
            });
            continue;
        }
        let abs = root.join(path);
        let existing = fs::symlink_metadata(&abs).ok();
        if existing.is_some() && !current.contains_key(path) && oracle.is_excluded(path) {
            out.skipped.push(Skipped {
                path: path.clone(),
                reason: "ignored, a secret or too large now; left untouched".into(),
            });
            continue;
        }
        match write_entry(store, &abs, *entry, existing.as_ref()) {
            Ok(()) => out.written.push(path.clone()),
            Err(reason) => out.skipped.push(Skipped {
                path: path.clone(),
                reason,
            }),
        }
    }
    Ok(out)
}

pub fn write_entry(
    store: &Store,
    abs: &Path,
    entry: Entry,
    existing: Option<&fs::Metadata>,
) -> std::result::Result<(), String> {
    if existing.is_some_and(|m| m.is_dir()) {
        return Err("a directory is in the way".into());
    }
    let content = odb::read_blob(store.repo().map_err(|e| e.to_string())?, entry.oid)
        .map_err(|e| e.to_string())?;
    let parent = abs.parent().ok_or("no parent directory")?;
    ensure_parent_inside(&store.root, parent)?;
    fs::create_dir_all(parent).map_err(|e| format!("could not create directory: {e}"))?;
    if entry.mode == MODE_LINK {
        return symlink_atomic(Path::new(OsStr::from_bytes(&content)), abs)
            .map_err(|e| format!("could not link: {e}"));
    }
    let base = existing
        .filter(|m| m.is_file())
        .map(|m| m.permissions().mode() & 0o7777)
        .unwrap_or(0o644);
    let mode = if entry.mode == MODE_EXEC {
        base | ((base & 0o444) >> 2) | 0o100
    } else {
        base & !0o111
    };
    write_atomic(abs, &content, Some(fs::Permissions::from_mode(mode)))
        .map_err(|e| format!("could not write: {e}"))
}

/// Refuses to write through a symlinked directory that leads outside the root.
fn ensure_parent_inside(root: &Path, parent: &Path) -> std::result::Result<(), String> {
    let mut existing = parent;
    while !existing.exists() {
        existing = existing.parent().ok_or("no existing parent directory")?;
    }
    let canonical = existing
        .canonicalize()
        .map_err(|e| format!("could not resolve parent: {e}"))?;
    if canonical.starts_with(root) {
        Ok(())
    } else {
        Err("parent directory leads outside the project".into())
    }
}
