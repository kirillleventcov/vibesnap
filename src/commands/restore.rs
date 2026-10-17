use crate::error::Result;
use crate::vibe::{
    apply::{RestoreOutcome, restore_to},
    checkpoint::find_checkpoint,
    lock::lock_repo,
    paths::resolve_user_paths,
    repo::Repo,
};
use colored::*;
use std::path::PathBuf;

pub fn restore_command(checkpoint_id: String, files: Vec<PathBuf>, json: bool) -> Result<()> {
    let mut repo = Repo::open()?;
    let scope = resolve_user_paths(&repo.root, &files)?;
    let scope: Vec<String> = if scope.iter().any(|p| p.is_empty()) {
        Vec::new()
    } else {
        scope
    };
    let target = find_checkpoint(&repo.conn, &checkpoint_id)?;

    let lock = lock_repo(&repo.root)?;
    let outcome = restore_to(&mut repo, &lock, &target, &scope, None)?;
    print_restore_outcome(&outcome, json, None);
    Ok(())
}

pub fn print_restore_outcome(outcome: &RestoreOutcome, json: bool, track: Option<&str>) {
    if json {
        let mut out = serde_json::to_value(outcome).unwrap_or_default();
        out["v"] = 1.into();
        if let Some(track) = track {
            out["track"] = track.into();
        }
        println!("{out}");
        return;
    }

    println!(
        "Restored checkpoint {}: {} written, {} deleted, {} unchanged",
        outcome.target.green(),
        outcome.written.len(),
        outcome.deleted.len(),
        outcome.unchanged
    );
    if let Some(saved) = &outcome.saved {
        println!(
            "Saved the previous state as checkpoint {} (restore it to undo)",
            saved.cyan()
        );
    }
    for skipped in &outcome.skipped {
        eprintln!(
            "{}",
            format!("Skipped {}: {}", skipped.path, skipped.reason).yellow()
        );
    }
    if !outcome.deletion_enabled {
        eprintln!(
            "{}",
            format!(
                "Note: checkpoint {} may not cover the whole tree (partial, or made by an older version), so no files were deleted.",
                outcome.target
            )
            .yellow()
        );
    }
}
