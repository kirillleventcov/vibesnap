use crate::error::{AppError, Result};
use crate::vibe::{constants::REPO_DIRNAME, repo::find_repo_root};
use colored::*;
use std::fs;

pub fn reset_command(confirm: bool) -> Result<()> {
    if !confirm {
        let confirmation = dialoguer::Confirm::new()
            .with_prompt(
                "This will irreversibly delete the .vibe directory and all snaps. Are you sure?",
            )
            .interact()
            .map_err(|e| AppError::DialoguerError(e))?;

        if !confirmation {
            println!("Reset cancelled.");
            return Ok(());
        }
    }

    match find_repo_root(None) {
        Ok(root) => {
            let vibe_dir = root.join(REPO_DIRNAME);
            if vibe_dir.exists() {
                fs::remove_dir_all(&vibe_dir)?;
                println!("{}", "VibeSnap repo has been reset.".green());
            } else {
                println!("No VibeSnap repo found to reset.");
            }
        }
        Err(AppError::NotInRepo) => {
            println!("No VibeSnap repo found to reset.");
        }
        Err(e) => return Err(e),
    }

    Ok(())
}
