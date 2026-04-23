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

    restore_files_from_manifest(&root, &filtered_manifest)?;

    if selective_files.is_none() {
        write_head(&root, &current_track_name, Some(&checkpoint_id))?;
    }

    let files_info = if selective_files.is_some() {
        format!(
            " ({} files)",
            filtered_manifest.files.len()
        )
    } else {
        format!(" ({} files)", filtered_manifest.files.len())
    };

    println!(
        "{}",
        format!(
            "Restored{}{} from checkpoint {}",
            if selective_files.is_some() { " files" } else { "" },
            files_info,
            checkpoint_id.green()
        )
    );

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
