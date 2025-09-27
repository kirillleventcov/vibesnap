use crate::config::Config;
use crate::error::Result;
use crate::vibe::{
    db::db_connect,
    repo::{find_repo_root, read_head, write_head},
    snapshot::{build_snapshot_manifest, save_snapshot_manifest},
    utils::generate_id,
};
use colored::*;
use std::path::PathBuf;

pub fn snap_command(
    paths: Vec<PathBuf>,
    note: String,
    show_progress: bool,
    selective_files: Option<Vec<PathBuf>>,
) -> Result<()> {
    let config = Config::load();
    let root = find_repo_root(None)?;
    let (track, parent_id) = read_head(&root)?;

    let conn = db_connect(&root)?;

    let checkpoint_id = generate_id(&track);

    // Use auto-note if no note provided
    let final_note = if note.is_empty() {
        config.format_auto_note()
    } else {
        note
    };

    // Determine what to snap
    let paths_to_process_input = match &selective_files {
        Some(files) => files.clone(),
        None => {
            if paths.is_empty() || (paths.len() == 1 && paths[0] == PathBuf::from(".")) {
                vec![PathBuf::from(".")]
            } else {
                paths
            }
        }
    };

    // Use config to determine if progress should be shown
    let should_show_progress = config.should_show_progress(show_progress);

    // Build manifest with progress bar based on config
    let manifest = if should_show_progress {
        crate::cli::progress::build_snapshot_manifest_with_progress(&root, &paths_to_process_input)?
    } else {
        build_snapshot_manifest(&root, &paths_to_process_input)?
    };

    // Save the manifest
    save_snapshot_manifest(&root, &checkpoint_id, &manifest)?;

    conn.execute(
        "INSERT INTO checkpoints(id, track, parent, timestamp, note) VALUES (?, ?, ?, ?, ?)",
        rusqlite::params![
            checkpoint_id,
            track,
            parent_id,
            chrono::Utc::now().timestamp(),
            final_note
        ],
    )?;

    conn.execute(
        "UPDATE tracks SET head = ? WHERE name = ?",
        rusqlite::params![checkpoint_id, track],
    )?;

    write_head(&root, &track, Some(&checkpoint_id))?;

    let file_count = manifest.files.len();
    let files_info = if let Some(ref selective) = selective_files {
        format!(
            " ({} files: {})",
            file_count,
            selective
                .iter()
                .map(|p| p.to_string_lossy())
                .collect::<Vec<_>>()
                .join(", ")
        )
    } else {
        format!(" ({} files)", file_count)
    };

    println!(
        "{}",
        format!("✓ snap {}{} - {}", checkpoint_id, files_info, final_note).green()
    );

    Ok(())
}
