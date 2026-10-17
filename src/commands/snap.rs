use crate::config::Config;
use crate::error::{AppError, Result};
use crate::vibe::{
    checkpoint::{manifest_covers, record_state},
    lock::lock_repo,
    paths::resolve_user_paths,
    repo::Repo,
    snapshot::load_snapshot_manifest,
};
use colored::*;
use std::path::PathBuf;

pub fn snap_command(paths: Vec<PathBuf>, note: String, json: bool) -> Result<()> {
    let config = Config::load();
    let mut repo = Repo::open()?;

    let scope = resolve_user_paths(&repo.root, &paths)?;
    // A path that resolves to the repo root means the whole tree.
    let scope: Vec<String> = if scope.iter().any(|p| p.is_empty()) {
        Vec::new()
    } else {
        scope
    };

    // Use auto-note if no note provided
    let final_note = if note.is_empty() {
        config.format_auto_note()
    } else {
        note
    };

    let lock = lock_repo(&repo.root)?;

    if !scope.is_empty() {
        let head_manifest = match repo.head()?.checkpoint {
            Some(id) => Some(load_snapshot_manifest(&repo.root, &id)?),
            None => None,
        };
        for path in &scope {
            let known = head_manifest
                .as_ref()
                .is_some_and(|m| manifest_covers(m, path));
            if !repo.root.join(path).exists() && !known {
                return Err(AppError::Usage(format!("Path not found: {path}")));
            }
        }
    }

    let recorded = record_state(&mut repo, &lock, &scope, &final_note)?;
    let file_count = recorded.manifest.files.len();

    if json {
        let mut out = serde_json::json!({
            "v": 1,
            "created": recorded.created,
            "id": recorded.id,
            "parent": recorded.parent,
            "track": recorded.track,
            "files": file_count,
            "complete": recorded.complete,
            "paths": scope,
        });
        if recorded.created {
            out["note"] = final_note.into();
        }
        println!("{out}");
        return Ok(());
    }

    if !recorded.created {
        println!(
            "Nothing changed since {}",
            recorded.id.as_deref().unwrap_or_default()
        );
        return Ok(());
    }

    let files_info = if scope.is_empty() {
        format!(" ({} files)", file_count)
    } else {
        format!(" ({} files; re-read: {})", file_count, scope.join(", "))
    };
    println!(
        "{}",
        format!(
            "snap {}{} - {}",
            recorded.id.as_deref().unwrap_or_default(),
            files_info,
            final_note
        )
        .green()
    );
    if !recorded.complete {
        eprintln!(
            "{}",
            "Note: HEAD does not cover the whole tree, so this checkpoint holds only part of it. Run 'vibesnap snap' with no paths to record everything."
                .yellow()
        );
    }

    Ok(())
}
