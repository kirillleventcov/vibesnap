use crate::commands::{diff::diff_command, restore::restore_command, switch::switch_command};
use crate::error::{AppError, Result};
use crate::vibe::db::db_connect;
use crate::vibe::repo::find_repo_root;
use crate::vibe::snapshot::load_snapshot_manifest;
use chrono::{Local, TimeZone};
use dialoguer::{Select, theme::ColorfulTheme};
use std::path::PathBuf;

use super::display::display_file_tree;

pub fn interactive_list_selection(
    checkpoints: Vec<(String, String, Option<String>, i64, Option<String>)>,
) -> Result<()> {
    let root = find_repo_root(None)?;
    let items: Vec<String> = checkpoints
        .iter()
        .map(|(id, track, _, timestamp, note)| {
            let local_datetime = Local
                .timestamp_opt(*timestamp, 0)
                .single()
                .unwrap_or_default();
            format!(
                "{} ({}) - {} - {}",
                id,
                track,
                local_datetime.format("%Y-%m-%d %H:%M:%S"),
                note.as_deref().unwrap_or("No note")
            )
        })
        .collect();

    let selection = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("Select a checkpoint")
        .default(0)
        .items(&items)
        .interact()?;

    let (selected_id, _, _, _, _) = &checkpoints[selection];

    let actions = vec!["Restore", "Show Details", "Cancel"];
    let action = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("What would you like to do?")
        .default(0)
        .items(&actions)
        .interact()?;

    match action {
        0 => restore_command(selected_id.to_string(), true, None)?,
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

pub fn interactive_restore_command(
    show_progress: bool,
    selective_files: Option<Vec<PathBuf>>,
) -> Result<()> {
    let root = find_repo_root(None)?;
    let conn = db_connect(&root)?;

    let mut stmt = conn.prepare(
        "SELECT id, track, parent, timestamp, note FROM checkpoints ORDER BY timestamp DESC",
    )?;
    let checkpoint_iter = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, i64>(3)?,
            row.get::<_, Option<String>>(4)?,
        ))
    })?;

    let checkpoints: Vec<_> = checkpoint_iter.collect::<std::result::Result<_, _>>()?;

    if checkpoints.is_empty() {
        println!("No checkpoints found.");
        return Ok(());
    }

    let items: Vec<String> = checkpoints
        .iter()
        .map(|(id, track, _, timestamp, note)| {
            let local_datetime = Local
                .timestamp_opt(*timestamp, 0)
                .single()
                .unwrap_or_default();
            format!(
                "{} ({}) - {} - {}",
                id,
                track,
                local_datetime.format("%Y-%m-%d %H:%M:%S"),
                note.as_deref().unwrap_or("No note")
            )
        })
        .collect();

    let selection = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("Select checkpoint to restore")
        .default(0)
        .items(&items)
        .interact()?;

    let (selected_id, _, _, _, _) = &checkpoints[selection];
    restore_command(selected_id.to_string(), show_progress, selective_files)
}

pub fn interactive_file_restore_command(checkpoint_id: String, show_progress: bool) -> Result<()> {
    let root = find_repo_root(None)?;
    let manifest = load_snapshot_manifest(&root, &checkpoint_id)?;

    if manifest.files.is_empty() {
        println!("No files found in checkpoint {}", checkpoint_id);
        return Ok(());
    }

    let mut file_list: Vec<String> = manifest.files.keys().cloned().collect();
    file_list.sort();

    let selection = Select::with_theme(&ColorfulTheme::default())
        .with_prompt(format!(
            "Select file to restore from checkpoint {}",
            checkpoint_id
        ))
        .default(0)
        .items(&file_list)
        .interact()?;

    let selected_file = PathBuf::from(&file_list[selection]);
    restore_command(checkpoint_id, show_progress, Some(vec![selected_file]))
}

pub fn interactive_switch_command() -> Result<()> {
    let root = find_repo_root(None)?;
    let conn = db_connect(&root)?;

    let mut stmt = conn.prepare("SELECT name FROM tracks ORDER BY name")?;
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

    switch_command(tracks[selection].clone())
}

pub fn interactive_diff_command(file_path_opt: Option<PathBuf>, side_by_side: bool) -> Result<()> {
    let root = find_repo_root(None)?;
    let conn = db_connect(&root)?;

    let mut stmt = conn.prepare(
        "SELECT id, track, parent, timestamp, note FROM checkpoints ORDER BY timestamp DESC",
    )?;
    let checkpoints: Vec<_> = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, Option<String>>(4)?,
            ))
        })?
        .collect::<std::result::Result<_, _>>()?;

    if checkpoints.len() < 2 {
        return Err(AppError::NotEnoughCheckpointsForDiff);
    }

    let items: Vec<String> = checkpoints
        .iter()
        .map(|(id, track, _, timestamp, note)| {
            let local_datetime = Local
                .timestamp_opt(*timestamp, 0)
                .single()
                .unwrap_or_default();
            format!(
                "{} ({}) - {} - {}",
                id,
                track,
                local_datetime.format("%Y-%m-%d %H:%M:%S"),
                note.as_deref().unwrap_or("No note")
            )
        })
        .collect();

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

    let (id1, _, _, _, _) = &checkpoints[selection1];
    let (id2, _, _, _, _) = &checkpoints[selection2];

    diff_command(id1.clone(), id2.clone(), file_path_opt, side_by_side)
}
