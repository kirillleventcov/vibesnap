use crate::error::{AppError, Result};
use crate::vibe::{
    db::db_connect,
    repo::{find_repo_root, read_head},
};
use colored::*;

pub fn fastforward_command(progress: bool) -> Result<()> {
    let root = find_repo_root(None)?;
    let conn = db_connect(&root)?;
    let (current_track, current_checkpoint_id) = read_head(&root)?;

    let current_checkpoint_id = current_checkpoint_id.ok_or_else(|| {
        AppError::Generic("No current checkpoint to fast-forward from".to_string())
    })?;

    // Get current checkpoint timestamp
    let current_timestamp: i64 = conn
        .query_row(
            "SELECT timestamp FROM checkpoints WHERE id = ?",
            rusqlite::params![current_checkpoint_id],
            |row| row.get(0),
        )
        .map_err(|_| {
            AppError::Generic(format!(
                "Could not find current checkpoint: {}",
                current_checkpoint_id
            ))
        })?;

    // Find next checkpoint on current track after current timestamp
    let next_checkpoint_id: String = conn
        .query_row(
            "SELECT id FROM checkpoints
             WHERE track = ? AND timestamp > ?
             ORDER BY timestamp ASC LIMIT 1",
            rusqlite::params![current_track, current_timestamp],
            |row| row.get(0),
        )
        .map_err(|_| {
            AppError::Generic(
                "No checkpoint found ahead of current position. You're at the latest!".to_string(),
            )
        })?;

    println!(
        "{}",
        format!("Fast-forwarding to checkpoint {} ...", next_checkpoint_id).cyan()
    );

    crate::commands::restore::restore_command(next_checkpoint_id, progress, None)?;

    Ok(())
}
