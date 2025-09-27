use crate::error::Result;
use crate::vibe::{
    db::db_connect,
    repo::{find_repo_root, read_head},
};
use colored::*;

use super::restore::restore_command;

pub fn latest_command(show_progress: bool) -> Result<()> {
    let root = find_repo_root(None)?;
    let (track, _current_head) = read_head(&root)?;

    let conn = db_connect(&root)?;
    let latest_head_id: rusqlite::Result<Option<String>> = conn.query_row(
        "SELECT head FROM tracks WHERE name = ?1",
        rusqlite::params![&track],
        |row| row.get(0),
    );

    match latest_head_id {
        Ok(Some(head_id)) => {
            restore_command(head_id.clone(), show_progress, None)?;
            println!(
                "{}",
                format!(
                    "Restored latest checkpoint {} on track {}",
                    head_id.green(),
                    track.green()
                )
            );
        }
        Ok(None) => {
            println!(
                "{}",
                format!("Track {} has no checkpoints.", track).yellow()
            );
        }
        Err(e) => return Err(e.into()),
    }

    Ok(())
}
