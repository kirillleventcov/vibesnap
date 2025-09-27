use crate::error::{AppError, Result};
use colored::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir;

use super::constants::{REPO_DIRNAME, SNAPSHOTS_DIRNAME};
use super::ignore::{read_ignore_patterns, should_ignore_path};
use super::objects::store_content_in_objects;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SnapshotManifest {
    pub files: HashMap<String, String>, // path -> content hash
}

// Helper function to save snapshot manifest
pub fn save_snapshot_manifest(
    root: &Path,
    checkpoint_id: &str,
    manifest: &SnapshotManifest,
) -> Result<()> {
    let snapshots_dir: PathBuf = root.join(REPO_DIRNAME).join(SNAPSHOTS_DIRNAME);
    fs::create_dir_all(&snapshots_dir)?;

    let manifest_path: PathBuf = snapshots_dir.join(format!("{}.json", checkpoint_id));
    let json_content: String = serde_json::to_string_pretty(manifest)
        .map_err(|e| AppError::ManifestSerializationError(e.to_string()))?;

    fs::write(manifest_path, json_content).map_err(AppError::IoError)
}

// Helper function to load snapshot manifest
pub fn load_snapshot_manifest(root: &Path, checkpoint_id: &str) -> Result<SnapshotManifest> {
    let manifest_path: PathBuf = root
        .join(REPO_DIRNAME)
        .join(SNAPSHOTS_DIRNAME)
        .join(format!("{}.json", checkpoint_id));

    if !manifest_path.exists() {
        return Err(AppError::ManifestNotFound(checkpoint_id.to_string()));
    }

    let json_content: String = fs::read_to_string(manifest_path)?;
    serde_json::from_str(&json_content)
        .map_err(|e| AppError::ManifestDeserializationError(e.to_string()))
}

// Helper function to build snapshot manifest from paths
pub fn build_snapshot_manifest(
    root: &Path,
    paths_to_snap_relative_to_root_or_absolute: &[PathBuf],
) -> Result<SnapshotManifest> {
    let mut manifest: SnapshotManifest = SnapshotManifest {
        files: HashMap::new(),
    };

    // Read ignore patterns (prioritizing .gitignore over .vibeignore)
    let ignore_patterns = read_ignore_patterns(root)?;

    for p_user_input in paths_to_snap_relative_to_root_or_absolute {
        let path_to_process: PathBuf = if p_user_input.is_absolute() {
            p_user_input.clone()
        } else {
            root.join(p_user_input)
        };

        let source_path_canon: PathBuf = match path_to_process.canonicalize() {
            Ok(cp) => cp,
            Err(_e) => {
                eprintln!(
                    "{}",
                    format!(
                        "Warning: Path {} could not be canonicalized (e.g. non-existent or broken link) - skipped",
                        path_to_process.display()
                    )
                    .yellow()
                );
                continue;
            }
        };

        if !source_path_canon.exists() {
            eprintln!(
                "{}",
                format!(
                    "Warning: {} does not exist - skipped",
                    source_path_canon.display()
                )
                .yellow()
            );
            continue;
        }

        // Check if this path should be ignored
        if should_ignore_path(&source_path_canon, root, &ignore_patterns) {
            eprintln!(
                "{}",
                format!(
                    "Ignored: {} (matches .vibeignore pattern)",
                    source_path_canon.display()
                )
                .yellow()
            );
            continue;
        }

        let storage_rel_path: PathBuf = source_path_canon
            .strip_prefix(root)
            .map(|rel: &Path| rel.to_path_buf())
            .unwrap_or_else(|_| {
                PathBuf::from(
                    source_path_canon
                        .file_name()
                        .unwrap_or_else(|| source_path_canon.as_os_str()),
                )
            });

        if source_path_canon.is_dir() {
            for entry_result in walkdir::WalkDir::new(&source_path_canon) {
                let entry: walkdir::DirEntry = match entry_result {
                    Ok(e) => e,
                    Err(e) => {
                        eprintln!(
                            "{}",
                            format!(
                                "Warning: Error walking directory {}: {} - skipped entry",
                                source_path_canon.display(),
                                e
                            )
                            .yellow()
                        );
                        continue;
                    }
                };

                let entry_path: &Path = entry.path();

                // Check if this file should be ignored
                if should_ignore_path(entry_path, root, &ignore_patterns) {
                    continue; // Skip ignored files silently during directory traversal
                }

                if entry_path.is_file() {
                    let rel_to_source_dir: &Path =
                        entry_path.strip_prefix(&source_path_canon).unwrap();
                    let manifest_path: PathBuf = storage_rel_path.join(rel_to_source_dir);

                    match fs::read(entry_path) {
                        Ok(content) => match store_content_in_objects(root, &content) {
                            Ok(hash) => {
                                manifest
                                    .files
                                    .insert(manifest_path.to_string_lossy().to_string(), hash);
                            }
                            Err(e) => {
                                eprintln!(
                                    "{}",
                                    format!(
                                        "Warning: Failed to store {} - Error: {} - skipped",
                                        entry_path.display(),
                                        e
                                    )
                                    .yellow()
                                );
                            }
                        },
                        Err(e) => {
                            eprintln!(
                                "{}",
                                format!(
                                    "Warning: Failed to read {} - Error: {} - skipped",
                                    entry_path.display(),
                                    e
                                )
                                .yellow()
                            );
                        }
                    }
                }
            }
        } else if source_path_canon.is_file() {
            match fs::read(&source_path_canon) {
                Ok(content) => match store_content_in_objects(root, &content) {
                    Ok(hash) => {
                        manifest
                            .files
                            .insert(storage_rel_path.to_string_lossy().to_string(), hash);
                    }
                    Err(e) => {
                        eprintln!(
                            "{}",
                            format!(
                                "Warning: Failed to store {} - Error: {} - skipped",
                                source_path_canon.display(),
                                e
                            )
                            .yellow()
                        );
                    }
                },
                Err(e) => {
                    eprintln!(
                        "{}",
                        format!(
                            "Warning: Failed to read {} - Error: {} - skipped",
                            source_path_canon.display(),
                            e
                        )
                        .yellow()
                    );
                }
            }
        }
    }

    Ok(manifest)
}
