use crate::commands::{
    diff::diff_command, format_timestamp, restore::restore_command, switch::switch_command,
};
use crate::error::{AppError, Result};
use crate::vibe::checkpoint::{Checkpoint, list_checkpoints};
use crate::vibe::repo::Repo;
use crate::vibe::snapshot::load_snapshot_manifest;
use dialoguer::{Select, theme::ColorfulTheme};
use std::path::PathBuf;

use super::display::display_file_tree;

fn checkpoint_items(checkpoints: &[Checkpoint]) -> Vec<String> {
    checkpoints
        .iter()
        .map(|c| {
            format!(
                "{} ({}) - {} - {}",
                c.id,
                c.track,
                format_timestamp(c.timestamp),
                c.note.as_deref().unwrap_or("No note")
            )
        })
        .collect()
}

/// All checkpoints, newest first.
fn newest_first() -> Result<Vec<Checkpoint>> {
    let repo = Repo::open()?;
    let mut checkpoints = list_checkpoints(&repo.conn, None)?;
    checkpoints.reverse();
    Ok(checkpoints)
}

pub fn interactive_list_selection(checkpoints: Vec<Checkpoint>) -> Result<()> {
    let root = Repo::open()?.root;
    let items = checkpoint_items(&checkpoints);

    let selection = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("Select a checkpoint")
        .default(0)
        .items(&items)
        .interact()?;

    let selected_id = &checkpoints[selection].id;

    let actions = vec!["Restore", "Show Details", "Cancel"];
    let action = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("What would you like to do?")
        .default(0)
        .items(&actions)
        .interact()?;

    match action {
        0 => restore_command(selected_id.to_string(), Vec::new(), false)?,
        1 => {
            if let Ok(manifest) = load_snapshot_manifest(&root, selected_id) {
                println!("\nCheckpoint: {}", selected_id);
                display_file_tree(&manifest, "");
            }
        }
        _ => println!("Cancelled."),
    }

    Ok(())
}

pub fn interactive_restore_command(selective_files: Vec<PathBuf>) -> Result<()> {
    let checkpoints = newest_first()?;

    if checkpoints.is_empty() {
        println!("No checkpoints found.");
        return Ok(());
    }

    let items = checkpoint_items(&checkpoints);

    let selection = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("Select checkpoint to restore")
        .default(0)
        .items(&items)
        .interact()?;

    restore_command(checkpoints[selection].id.clone(), selective_files, false)
}

pub fn interactive_switch_command() -> Result<()> {
    let repo = Repo::open()?;
    let mut stmt = repo.conn.prepare("SELECT name FROM tracks ORDER BY name")?;
    let tracks: Vec<String> = stmt
        .query_map([], |row| row.get(0))?
        .collect::<std::result::Result<_, _>>()?;

    if tracks.is_empty() {
        println!("No tracks found.");
        return Ok(());
    }

    let selection = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("Select track to switch to")
        .default(0)
        .items(&tracks)
        .interact()?;

    switch_command(tracks[selection].clone(), false)
}

pub fn interactive_diff_command(file_path_opt: Option<PathBuf>) -> Result<()> {
    let checkpoints = newest_first()?;

    if checkpoints.len() < 2 {
        return Err(AppError::NotEnoughCheckpointsForDiff);
    }

    let items = checkpoint_items(&checkpoints);

    let selection1 = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("Select first checkpoint")
        .default(0)
        .items(&items)
        .interact()?;

    let selection2 = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("Select second checkpoint")
        .default(if selection1 < items.len() - 1 {
            selection1 + 1
        } else {
            0
        })
        .items(&items)
        .interact()?;

    diff_command(
        checkpoints[selection1].id.clone(),
        checkpoints[selection2].id.clone(),
        file_path_opt,
        false,
    )
}
