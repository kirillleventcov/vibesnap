use crate::error::{AppError, Result};
use crate::is_interactive_terminal;
use crate::vibe::{constants::REPO_DIRNAME, lock::lock_repo, repo::find_repo_root};
use colored::*;
use std::fs;

pub fn reset_command(confirm: bool, json: bool) -> Result<()> {
    if !confirm {
        if json || !is_interactive_terminal() {
            return Err(AppError::Usage(
                "Refusing to delete .vibe without --confirm".into(),
            ));
        }
        let confirmation = dialoguer::Confirm::new()
            .with_prompt(
                "This will irreversibly delete the .vibe directory and all snaps. Are you sure?",
            )
            .interact()?;

        if !confirmation {
            println!("Reset cancelled.");
            return Ok(());
        }
    }

    let removed = match find_repo_root(None) {
        Ok(root) => {
            let vibe_dir = root.join(REPO_DIRNAME);
            // Wait for any running writer before deleting the store under it.
            let _lock = lock_repo(&root)?;
            fs::remove_dir_all(&vibe_dir)?;
            true
        }
        Err(AppError::NotInRepo) => false,
        Err(e) => return Err(e),
    };

    if json {
        println!("{}", serde_json::json!({"v": 1, "removed": removed}));
    } else if removed {
        println!("{}", "VibeSnap repo has been reset.".green());
    } else {
        println!("No VibeSnap repo found to reset.");
    }
    Ok(())
}
