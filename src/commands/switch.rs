use crate::error::{AppError, Result};
use crate::vibe::{
    db::db_connect,
    repo::{find_repo_root, write_head},
};
use colored::*;

use super::restore::restore_command;

pub fn switch_command(name: String) -> Result<()> {
    let root = find_repo_root(None)?;
    let conn = db_connect(&root)?;

    let query_result: rusqlite::Result<Option<String>> = conn.query_row(
        "SELECT head FROM tracks WHERE name = ?1",
        rusqlite::params![&name],
        |row| row.get(0),
    );

    match query_result {
        Ok(Some(head_id)) => {
            restore_command(head_id.clone(), false, None)?;
            write_head(&root, &name, Some(&head_id))?;
            println!(
                "{}",
                format!(
                    "Switched to track {} and restored checkpoint {}",
                    name.green(),
                    head_id.green()
                )
            );
            Ok(())
        }
        Ok(None) => {
            write_head(&root, &name, None)?;
            println!(
                "{}",
                format!("Switched to empty track {}", name.green()).yellow()
            );
            Ok(())
        }
        Err(rusqlite::Error::QueryReturnedNoRows) => Err(AppError::TrackNotFound(name)),
        Err(e) => Err(e.into()),
    }
}
