use crate::error::{AppError, Result};
use crate::vibe::{
    apply::restore_to,
    checkpoint::find_checkpoint,
    lock::lock_repo,
    repo::{Repo, set_head},
};
use colored::*;
use rusqlite::OptionalExtension;

use super::restore::print_restore_outcome;

pub fn switch_command(name: String, json: bool) -> Result<()> {
    let mut repo = Repo::open()?;
    let lock = lock_repo(&repo.root)?;

    let head: Option<Option<String>> = repo
        .conn
        .query_row(
            "SELECT head FROM tracks WHERE name = ?1",
            rusqlite::params![&name],
            |row| row.get(0),
        )
        .optional()?;

    match head {
        Some(Some(head_id)) => {
            let target = find_checkpoint(&repo.conn, &head_id)?;
            let outcome = restore_to(&mut repo, &lock, &target, &[], Some(&name))?;
            print_restore_outcome(&outcome, json, Some(&name));
            if !json {
                println!("Switched to track {}", name.green());
            }
            Ok(())
        }
        Some(None) => {
            set_head(&repo.conn, &name, None)?;
            if json {
                println!(
                    "{}",
                    serde_json::json!({"v": 1, "track": name, "target": null})
                );
            } else {
                println!(
                    "{}",
                    format!("Switched to empty track {}", name.green()).yellow()
                );
            }
            Ok(())
        }
        None => Err(AppError::TrackNotFound(name)),
    }
}
