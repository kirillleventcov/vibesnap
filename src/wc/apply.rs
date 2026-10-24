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
use std::collections::{BTreeMap, HashMap, HashSet};
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

/// Decides whether a path may be written or deleted, before anything changes:
/// deciding as you go would let deleting a `.gitignore` early unprotect the
/// files it ignores.
pub fn blocked(
    root: &Path,
    oracle: &mut Oracle,
    protected: &HashSet<String>,
    path: &str,
) -> Option<String> {
    if !is_safe_path(path) {
        return Some("unsafe path".into());
    }
    if protected.contains(path) {
        return Some(
            "could not be read when the current state was recorded; left untouched".into(),
        );
    }
    if parent_is_symlink(root, path) {
        return Some("a parent directory is a symlink; left untouched".into());
    }
    if fs::symlink_metadata(root.join(path)).is_ok() && oracle.is_excluded(path) {
        return Some("ignored, a secret or too large; left untouched".into());
    }
    None
}

/// True if any directory between the root and `path` is a symlink. Recorded
/// paths never go through one (the walker does not follow them), so writing
/// through one could only land somewhere unrecorded, possibly inside `.git`.
fn parent_is_symlink(root: &Path, path: &str) -> bool {
    let mut dir = root.to_path_buf();
    let parts: Vec<&str> = path.split('/').collect();
    for part in &parts[..parts.len().saturating_sub(1)] {
        dir.push(part);
        match fs::symlink_metadata(&dir) {
            Ok(meta) if meta.file_type().is_symlink() => return true,
            Ok(_) => {}
            Err(_) => return false,
        }
    }
    false
}

/// Brings the files under `scope` from `current` to `target`. `current` must be
/// the recorded state of the working tree right now (the caller records it
/// first); `protected` are files that could not be read then. Deletes only when
/// `delete` is set.
pub fn apply(
    store: &Store,
    oracle: &mut Oracle,
    protected: &HashSet<String>,
    current: &BTreeMap<String, Entry>,
    target: &BTreeMap<String, Entry>,
    scope: &[String],
    delete: bool,
) -> Result<Applied> {
    let mut out = Applied::default();
    let root = &store.root;

    let deletions: Vec<&String> = if delete {
        current
            .keys()
            .filter(|p| in_scope(p, scope) && !target.contains_key(*p))
            .collect()
    } else {
        Vec::new()
    };
    let writes: Vec<(&String, &Entry)> = target
        .iter()
        .filter(|(p, e)| in_scope(p, scope) && current.get(*p) != Some(*e))
        .collect();
    out.unchanged = target
        .iter()
        .filter(|(p, e)| in_scope(p, scope) && current.get(*p) == Some(*e))
        .count();
    let mut verdicts: HashMap<&str, Option<String>> = HashMap::new();
    for path in deletions
        .iter()
        .copied()
        .chain(writes.iter().map(|(p, _)| *p))
    {
        verdicts.insert(path, blocked(root, oracle, protected, path));
    }

    for path in deletions {
        if let Some(Some(reason)) = verdicts.get(path.as_str()) {
            out.skipped.push(Skipped {
                path: path.clone(),
                reason: reason.clone(),
            });
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

    for (path, entry) in writes {
        if let Some(Some(reason)) = verdicts.get(path.as_str()) {
            out.skipped.push(Skipped {
                path: path.clone(),
                reason: reason.clone(),
            });
            continue;
        }
        let abs = root.join(path);
        let existing = fs::symlink_metadata(&abs).ok();
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

/// Refuses to write through a symlinked directory, or into `.git` or `.vibe`.
fn ensure_parent_inside(root: &Path, parent: &Path) -> std::result::Result<(), String> {
    let mut existing = parent;
    while !existing.exists() {
        existing = existing.parent().ok_or("no existing parent directory")?;
    }
    let canonical = existing
        .canonicalize()
        .map_err(|e| format!("could not resolve parent: {e}"))?;
    match canonical.strip_prefix(root) {
        Ok(rel) if !crate::paths::is_reserved(&crate::paths::to_slash(rel)) => {}
        Ok(_) => return Err("parent directory is inside .git or .vibe".into()),
        Err(_) => return Err("parent directory leads outside the project".into()),
    }
    let rel = parent
        .strip_prefix(root)
        .map_err(|_| "outside the project")?;
    if !rel.as_os_str().is_empty()
        && parent_is_symlink(root, &format!("{}/x", crate::paths::to_slash(rel)))
    {
        return Err("a parent directory is a symlink".into());
    }
    Ok(())
}
