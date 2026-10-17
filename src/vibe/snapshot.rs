use crate::error::{AppError, Result};
use colored::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use super::constants::{REPO_DIRNAME, SNAPSHOTS_DIRNAME};
use super::ignore::{is_ignored, read_ignore_patterns};
use super::objects::ObjectStore;
use super::paths::{in_scope, to_slash};

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq)]
pub struct SnapshotManifest {
    pub files: BTreeMap<String, String>, // path -> content hash
}

fn manifest_path(root: &Path, checkpoint_id: &str) -> PathBuf {
    root.join(REPO_DIRNAME)
        .join(SNAPSHOTS_DIRNAME)
        .join(format!("{}.json", checkpoint_id))
}

/// Writes the manifest for a new checkpoint. Returns `Ok(false)` without touching
/// anything if a manifest with this ID already exists, so an ID collision can never
/// overwrite an existing checkpoint.
pub fn write_new_manifest(
    root: &Path,
    checkpoint_id: &str,
    manifest: &SnapshotManifest,
) -> Result<bool> {
    let path = manifest_path(root, checkpoint_id);
    fs::create_dir_all(path.parent().unwrap())?;
    let json_content = serde_json::to_vec(manifest)
        .map_err(|e| AppError::Generic(format!("Failed to serialize manifest: {e}")))?;

    let mut file = match fs::File::create_new(&path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => return Ok(false),
        Err(e) => return Err(AppError::Io(e)),
    };
    if let Err(e) = file.write_all(&json_content) {
        drop(file);
        let _ = fs::remove_file(&path);
        return Err(AppError::Io(e));
    }
    Ok(true)
}

pub fn remove_manifest(root: &Path, checkpoint_id: &str) {
    let _ = fs::remove_file(manifest_path(root, checkpoint_id));
}

pub fn load_snapshot_manifest(root: &Path, checkpoint_id: &str) -> Result<SnapshotManifest> {
    let json_content = match fs::read_to_string(manifest_path(root, checkpoint_id)) {
        Ok(content) => content,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(AppError::CheckpointNotFound(checkpoint_id.to_string()));
        }
        Err(e) => return Err(AppError::Io(e)),
    };
    serde_json::from_str(&json_content)
        .map_err(|e| AppError::Generic(format!("Failed to deserialize manifest: {e}")))
}

/// Reads the working tree under `scope` (repo-relative prefixes; empty means the
/// whole tree), stores every file's content and returns the resulting manifest.
/// Ignored directories are pruned rather than walked, and `.vibe` and `.git` are
/// always skipped.
pub fn scan(root: &Path, scope: &[String], store: &ObjectStore) -> Result<SnapshotManifest> {
    let patterns = read_ignore_patterns(root)?;
    let mut manifest = SnapshotManifest::default();
    let whole_tree = [String::new()];
    let prefixes = if scope.is_empty() {
        &whole_tree[..]
    } else {
        scope
    };

    for prefix in prefixes {
        if !prefix.is_empty() && is_ignored(prefix, &patterns) {
            eprintln!(
                "{}",
                format!("Ignored: {} (matches ignore pattern)", prefix).yellow()
            );
            continue;
        }
        let path = root.join(prefix);
        // A missing path is simply absent from the result, which records its deletion.
        let Ok(meta) = fs::metadata(&path) else {
            continue;
        };
        if meta.is_file() {
            add_file(store, &mut manifest, &path, prefix.clone());
            continue;
        }
        if !meta.is_dir() {
            continue;
        }
        let walker = walkdir::WalkDir::new(&path)
            .into_iter()
            .filter_entry(|entry| match entry.path().strip_prefix(root) {
                Ok(rel) => rel.as_os_str().is_empty() || !is_ignored(&to_slash(rel), &patterns),
                Err(_) => false,
            });
        for entry_result in walker {
            let entry = match entry_result {
                Ok(e) => e,
                Err(e) => {
                    eprintln!(
                        "{}",
                        format!(
                            "Warning: Error walking directory {}: {} - skipped entry",
                            path.display(),
                            e
                        )
                        .yellow()
                    );
                    continue;
                }
            };
            let entry_path = entry.path();
            if entry_path.is_file() {
                let rel = to_slash(entry_path.strip_prefix(root).unwrap());
                add_file(store, &mut manifest, entry_path, rel);
            }
        }
    }
    Ok(manifest)
}

fn add_file(store: &ObjectStore, manifest: &mut SnapshotManifest, path: &Path, rel: String) {
    let result = fs::read(path)
        .map_err(AppError::Io)
        .and_then(|content| store.store(&content));
    match result {
        Ok(hash) => {
            manifest.files.insert(rel, hash);
        }
        Err(e) => {
            eprintln!(
                "{}",
                format!(
                    "Warning: Failed to store {} - Error: {} - skipped",
                    path.display(),
                    e
                )
                .yellow()
            );
        }
    }
}

/// The parent state with everything under `scope` replaced by `scanned`. This is
/// what a partial snap records: the files outside the given paths keep their
/// previous content instead of disappearing from the checkpoint.
pub fn overlay(
    parent: Option<&SnapshotManifest>,
    scope: &[String],
    scanned: &SnapshotManifest,
) -> SnapshotManifest {
    let mut files: BTreeMap<String, String> = parent
        .map(|m| {
            m.files
                .iter()
                .filter(|(path, _)| !in_scope(path, scope))
                .map(|(p, h)| (p.clone(), h.clone()))
                .collect()
        })
        .unwrap_or_default();
    files.extend(scanned.files.iter().map(|(p, h)| (p.clone(), h.clone())));
    SnapshotManifest { files }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(entries: &[(&str, &str)]) -> SnapshotManifest {
        SnapshotManifest {
            files: entries
                .iter()
                .map(|(p, h)| (p.to_string(), h.to_string()))
                .collect(),
        }
    }

    #[test]
    fn overlay_replaces_only_the_scope() {
        let parent = manifest(&[("a", "1"), ("src/x", "2"), ("src/y", "3"), ("srcz", "4")]);
        let scanned = manifest(&[("src/x", "5")]);
        let result = overlay(Some(&parent), &["src".to_string()], &scanned);
        // src/y was deleted on disk, srcz is outside the scope.
        assert_eq!(
            result,
            manifest(&[("a", "1"), ("src/x", "5"), ("srcz", "4")])
        );
    }

    #[test]
    fn manifest_ids_are_never_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let first = manifest(&[("a", "1")]);
        let second = manifest(&[("b", "2")]);
        assert!(write_new_manifest(dir.path(), "AAAA", &first).unwrap());
        assert!(!write_new_manifest(dir.path(), "AAAA", &second).unwrap());
        assert_eq!(load_snapshot_manifest(dir.path(), "AAAA").unwrap(), first);
    }
}
