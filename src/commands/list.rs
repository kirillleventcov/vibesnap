use crate::cli::display::{display_checkpoints_table, display_checkpoints_with_tree};
use crate::cli::interactive::interactive_list_selection;
use crate::error::Result;
use crate::vibe::{
    db::db_connect,
    repo::find_repo_root,
    snapshot::{SnapshotManifest, load_snapshot_manifest},
};
use colored::*;
use std::path::PathBuf;

pub fn list_checkpoints_command(
    track_filter: Option<String>,
    show_tree: bool,
    interactive: bool,
    file_filter: Option<PathBuf>,
) -> Result<()> {
    let root = find_repo_root(None)?;
    let conn = db_connect(&root)?;

    let mut query = "SELECT id, track, parent, timestamp, note FROM checkpoints".to_string();
    let mut params: Vec<String> = Vec::new();

    if let Some(track_name) = track_filter {
        query.push_str(" WHERE track = ?1");
        params.push(track_name);
    }
    query.push_str(" ORDER BY timestamp");

    let mut stmt = conn.prepare(&query)?;
    let checkpoint_iter = stmt.query_map(rusqlite::params_from_iter(params), |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, i64>(3)?,
            row.get::<_, Option<String>>(4)?,
        ))
    })?;

    let mut checkpoints = Vec::new();
    for checkpoint_result in checkpoint_iter {
        let checkpoint = checkpoint_result?;

        if let Some(ref file_path) = file_filter {
            let manifest: SnapshotManifest = match load_snapshot_manifest(&root, &checkpoint.0) {
                Ok(m) => m,
                Err(_) => continue,
            };

            let file_path_str = file_path.to_string_lossy().to_string();
            if !manifest.files.contains_key(&file_path_str) {
                continue;
            }
        }

        checkpoints.push(checkpoint);
    }

    if checkpoints.is_empty() {
        if let Some(ref file_path) = file_filter {
            println!(
                "No checkpoints found containing file: {}",
                file_path.display()
            );
        } else {
            println!("No checkpoints found.");
        }
        return Ok(());
    }

    if let Some(ref file_path) = file_filter {
        println!(
            "Checkpoints containing {}:",
            file_path.display().to_string().cyan()
        );
    }

    if interactive {
        interactive_list_selection(checkpoints)?;
    } else if show_tree {
        display_checkpoints_with_tree(&root, checkpoints);
    } else {
        display_checkpoints_table(checkpoints);
    }

    Ok(())
}
