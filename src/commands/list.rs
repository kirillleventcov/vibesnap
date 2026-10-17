use crate::cli::display::display_compact_checkpoints;
use crate::cli::interactive::interactive_list_selection;
use crate::error::Result;
use crate::vibe::{
    checkpoint::{list_checkpoints, manifest_covers},
    paths::resolve_user_path,
    repo::Repo,
    snapshot::load_snapshot_manifest,
};
use colored::*;
use std::path::PathBuf;

pub fn list_checkpoints_command(
    track_filter: Option<String>,
    interactive: bool,
    file_filter: Option<PathBuf>,
    json: bool,
) -> Result<()> {
    let repo = Repo::open()?;
    let current_checkpoint_id = repo.head()?.checkpoint;
    let file_rel = match &file_filter {
        Some(path) => Some(resolve_user_path(&repo.root, path)?),
        None => None,
    };

    let mut checkpoints = list_checkpoints(&repo.conn, track_filter.as_deref())?;
    if let Some(file_rel) = &file_rel {
        checkpoints.retain(|c| {
            load_snapshot_manifest(&repo.root, &c.id).is_ok_and(|m| manifest_covers(&m, file_rel))
        });
    }

    if json {
        let items: Vec<serde_json::Value> = checkpoints
            .iter()
            .map(|c| {
                let mut item = serde_json::to_value(c).unwrap_or_default();
                item["current"] = (Some(&c.id) == current_checkpoint_id.as_ref()).into();
                item
            })
            .collect();
        println!(
            "{}",
            serde_json::json!({"v": 1, "head": current_checkpoint_id, "checkpoints": items})
        );
        return Ok(());
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
    } else {
        display_compact_checkpoints(&checkpoints, current_checkpoint_id);
    }

    Ok(())
}
