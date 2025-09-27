use crate::error::Result;
use crate::vibe::{
    objects::read_content_from_objects,
    repo::{find_repo_root, read_head, write_head},
    snapshot::{SnapshotManifest, load_snapshot_manifest},
};
use colored::*;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
pub fn restore_command(
    checkpoint_id: String,
    show_progress: bool,
    selective_files: Option<Vec<PathBuf>>,
) -> Result<()> {
    let root = find_repo_root(None)?;
    let manifest = load_snapshot_manifest(&root, &checkpoint_id)?;
    let (current_track_name, _) = read_head(&root)?;
    let filtered_manifest = if let Some(ref files) = selective_files.as_ref() {
        let mut filtered = SnapshotManifest {
            files: HashMap::new(),
        };
        for file_path in files.iter() {
            let file_path_str = file_path.to_string_lossy().to_string();
            if let Some(hash) = manifest.files.get(&file_path_str) {
                filtered.files.insert(file_path_str, hash.clone());
            } else {
                eprintln!(
                    "{}",
                    format!(
                        "Warning: File '{}' not found in checkpoint {}",
                        file_path.display(),
                        checkpoint_id
                    )
                    .yellow()
                );
            }
        }
        if filtered.files.is_empty() {
            eprintln!("{}", "No specified files found in checkpoint".red());
            return Ok(());
        }
        filtered
    } else {
        manifest
    };
    if show_progress {
        crate::cli::progress::restore_files_from_manifest_with_progress(&root, &filtered_manifest)?;
    } else {
        restore_files_from_manifest(&root, &filtered_manifest)?;
    }
    if selective_files.is_none() {
        write_head(&root, &current_track_name, Some(&checkpoint_id))?;
    }
    let files_info = if selective_files.is_some() {
        format!(
            " ({} files: {})",
            filtered_manifest.files.len(),
            filtered_manifest
                .files
                .keys()
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        )
    } else {
        format!(" ({} files)", filtered_manifest.files.len())
    };
    if selective_files.is_some() {
        println!(
            "{}",
            format!(
                "Selective restore completed{}\nRestored files from checkpoint {} to workspace",
                files_info,
                checkpoint_id.green()
            )
        );
    } else {
        let hint = format!(
            "\n{}\n  vibesnap branch <new-track-name> {}\n  vibesnap switch <new-track-name>",
            "Hint: If you want to save new work from this point on a new track, run:".cyan(),
            checkpoint_id
        );
        println!(
            "{}",
            format!(
                "Workspace restored to {}{}\n{}: Workspace files now match '{}'.\nYou are still on track '{}', and HEAD pointer is updated to '{}' for parenting purposes.{}",
                checkpoint_id.green(),
                files_info,
                "Detached mode".yellow(),
                checkpoint_id,
                current_track_name,
                checkpoint_id,
                hint
            )
        );
    }
    Ok(())
}
fn restore_files_from_manifest(root: &std::path::Path, manifest: &SnapshotManifest) -> Result<()> {
    for (file_path, hash) in &manifest.files {
        let dest_path = root.join(file_path);
        if let Some(parent_dir) = dest_path.parent() {
            fs::create_dir_all(parent_dir)?;
        }
        match read_content_from_objects(root, hash) {
            Ok(content) => match fs::write(&dest_path, content) {
                Ok(_) => {}
                Err(e) => {
                    eprintln!(
                        "{}",
                        format!(
                            "Warning: Failed to restore {} from object {}. Error: {} - skipped",
                            dest_path.display(),
                            hash,
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
                        "Warning: Failed to read object {} for file {}. Error: {} - skipped",
                        hash, file_path, e
                    )
                    .yellow()
                );
            }
        }
    }
    Ok(())
}
