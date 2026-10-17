use crate::error::{AppError, Result};
use colored::*;
use serde::Serialize;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use super::checkpoint::{Checkpoint, manifest_covers, record_state};
use super::fsutil::write_atomic;
use super::ignore::{is_ignored, read_ignore_patterns};
use super::lock::RepoLock;
use super::objects::read_content_from_objects;
use super::paths::{in_scope, is_safe_manifest_path};
use super::repo::{Repo, set_head};
use super::snapshot::load_snapshot_manifest;

#[derive(Debug, Serialize)]
pub struct Skipped {
    pub path: String,
    pub reason: String,
}

#[derive(Debug, Serialize)]
pub struct RestoreOutcome {
    pub target: String,
    /// The checkpoint that saved the working tree before anything was touched, if
    /// the tree differed from HEAD.
    pub saved: Option<String>,
    pub written: Vec<String>,
    pub deleted: Vec<String>,
    pub unchanged: usize,
    pub skipped: Vec<Skipped>,
    /// False for checkpoints that may not describe the whole tree (partial or
    /// legacy); then files missing from the checkpoint are left alone.
    pub deletion_enabled: bool,
    pub head_moved: bool,
}

/// Brings the working tree (or the part of it under `scope`) to `target`'s state.
///
/// The current state is recorded first, so the restore can itself be undone. Files
/// whose content already matches are not rewritten (their mtimes are kept), and
/// files the target does not have are deleted if the target is a whole-tree
/// checkpoint. Ignored files, `.vibe` and `.git` are never touched.
///
/// HEAD moves to the target after a whole-tree restore of a complete checkpoint, or
/// always when `switch_to_track` is given.
pub fn restore_to(
    repo: &mut Repo,
    lock: &RepoLock,
    target: &Checkpoint,
    scope: &[String],
    switch_to_track: Option<&str>,
) -> Result<RestoreOutcome> {
    let root = repo.root.clone();
    let target_manifest = load_snapshot_manifest(&root, &target.id)?;
    let deletion_enabled = target.complete == Some(true);
    let mut skipped = Vec::new();

    let mut wanted: BTreeMap<String, String> = BTreeMap::new();
    for (path, hash) in &target_manifest.files {
        if !in_scope(path, scope) {
            continue;
        }
        if is_safe_manifest_path(path) {
            wanted.insert(path.clone(), hash.clone());
        } else {
            skipped.push(Skipped {
                path: path.clone(),
                reason: "unsafe path in checkpoint (reserved directory or outside the repo)".into(),
            });
        }
    }

    if !scope.is_empty() {
        let mut usable = 0;
        for prefix in scope {
            if manifest_covers(&target_manifest, prefix)
                || (deletion_enabled && root.join(prefix).exists())
            {
                usable += 1;
            } else {
                eprintln!(
                    "{}",
                    format!(
                        "Warning: '{}' not found in checkpoint {}",
                        display_path(prefix),
                        target.id
                    )
                    .yellow()
                );
            }
        }
        if usable == 0 {
            return Err(AppError::Usage(format!(
                "None of the given paths are in checkpoint {}",
                target.id
            )));
        }
    }

    let recorded = record_state(
        repo,
        lock,
        scope,
        &format!("Saved before restoring {}", target.id),
    )?;
    let current = recorded.scanned.files;
    let saved = if recorded.created { recorded.id } else { None };
    let patterns = read_ignore_patterns(&root)?;

    let mut deleted = Vec::new();
    if deletion_enabled {
        for path in current.keys().filter(|p| !wanted.contains_key(*p)) {
            let abs = root.join(path);
            match fs::remove_file(&abs) {
                Ok(()) => {
                    remove_empty_parents(&root, &abs);
                    deleted.push(path.clone());
                }
                Err(e) => skipped.push(Skipped {
                    path: path.clone(),
                    reason: format!("could not delete: {e}"),
                }),
            }
        }
    }

    let mut written = Vec::new();
    let mut unchanged = 0;
    for (path, hash) in &wanted {
        if current.get(path) == Some(hash) {
            unchanged += 1;
            continue;
        }
        let abs = root.join(path);
        let existing = fs::symlink_metadata(&abs).ok();
        if existing.is_some() && !current.contains_key(path) && is_ignored(path, &patterns) {
            skipped.push(Skipped {
                path: path.clone(),
                reason: "matches an ignore pattern now; left untouched".into(),
            });
            continue;
        }
        match write_file(&root, &abs, hash, existing.as_ref()) {
            Ok(()) => written.push(path.clone()),
            Err(reason) => skipped.push(Skipped {
                path: path.clone(),
                reason,
            }),
        }
    }

    let head_moved =
        switch_to_track.is_some() || (scope.is_empty() && target.complete == Some(true));
    if head_moved {
        let track = match switch_to_track {
            Some(track) => track.to_string(),
            None => repo.head()?.track,
        };
        set_head(&repo.conn, &track, Some(&target.id))?;
    }

    Ok(RestoreOutcome {
        target: target.id.clone(),
        saved,
        written,
        deleted,
        unchanged,
        skipped,
        deletion_enabled,
        head_moved,
    })
}

fn write_file(
    root: &Path,
    abs: &Path,
    hash: &str,
    existing: Option<&fs::Metadata>,
) -> std::result::Result<(), String> {
    if existing.is_some_and(|m| m.is_dir()) {
        return Err("a directory is in the way".into());
    }
    let content = read_content_from_objects(root, hash).map_err(|e| e.to_string())?;
    let parent = abs.parent().ok_or("no parent directory")?;
    ensure_parent_inside(root, parent)?;
    fs::create_dir_all(parent).map_err(|e| format!("could not create directory: {e}"))?;
    // Keep the mode (e.g. the exec bit) of the file being replaced.
    let permissions = existing.filter(|m| m.is_file()).map(|m| m.permissions());
    write_atomic(parent, abs, &content, permissions).map_err(|e| format!("could not write: {e}"))
}

/// Refuses to write through a symlinked directory that leads outside the repo.
fn ensure_parent_inside(root: &Path, parent: &Path) -> std::result::Result<(), String> {
    let mut existing = parent;
    while !existing.exists() {
        existing = existing.parent().ok_or("no existing parent directory")?;
    }
    let canonical = existing
        .canonicalize()
        .map_err(|e| format!("could not resolve parent directory: {e}"))?;
    if canonical.starts_with(root) {
        Ok(())
    } else {
        Err("parent directory leads outside the repository".into())
    }
}

fn remove_empty_parents(root: &Path, file: &Path) {
    let mut dir = file.parent();
    while let Some(d) = dir {
        if d == root || !d.starts_with(root) || fs::remove_dir(d).is_err() {
            break;
        }
        dir = d.parent();
    }
}

fn display_path(path: &str) -> &str {
    if path.is_empty() { "." } else { path }
}
