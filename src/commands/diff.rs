use crate::cli::display::show_unified_diff;
use crate::error::{AppError, Result};
use crate::vibe::{
    checkpoint::{find_checkpoint, manifest_covers},
    objects::read_content_from_objects,
    paths::{in_scope, resolve_user_path},
    repo::Repo,
    snapshot::load_snapshot_manifest,
};
use colored::*;
use serde::Serialize;
use similar::{ChangeTag, TextDiff};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

#[derive(Serialize)]
struct FileChange {
    path: String,
    status: &'static str,
    binary: bool,
    added: usize,
    removed: usize,
}

pub fn diff_command(
    id1: String,
    id2: String,
    file_path_opt: Option<PathBuf>,
    json: bool,
) -> Result<()> {
    let repo = Repo::open()?;
    let root = &repo.root;
    let from = find_checkpoint(&repo.conn, &id1)?;
    let to = find_checkpoint(&repo.conn, &id2)?;
    let manifest1 = load_snapshot_manifest(root, &from.id)?;
    let manifest2 = load_snapshot_manifest(root, &to.id)?;

    let scope: Vec<String> = match &file_path_opt {
        Some(path) => {
            let path = resolve_user_path(root, path)?;
            if !path.is_empty()
                && !manifest_covers(&manifest1, &path)
                && !manifest_covers(&manifest2, &path)
            {
                return Err(AppError::Usage(format!(
                    "{path} is in neither checkpoint {} nor {}",
                    from.id, to.id
                )));
            }
            vec![path]
        }
        None => Vec::new(),
    };

    // A checkpoint that may not cover the whole tree says nothing about the files
    // it lacks, so those are not reported as added or deleted.
    let from_complete = from.complete == Some(true);
    let to_complete = to.complete == Some(true);

    let paths: BTreeSet<&String> = manifest1
        .files
        .keys()
        .chain(manifest2.files.keys())
        .filter(|p| in_scope(p, &scope))
        .collect();

    let mut changes = Vec::new();
    for path in paths {
        let hash1 = manifest1.files.get(path);
        let hash2 = manifest2.files.get(path);
        if hash1 == hash2
            || (hash1.is_none() && !from_complete)
            || (hash2.is_none() && !to_complete)
        {
            continue;
        }
        let content1 = read_optional(root, hash1)?;
        let content2 = read_optional(root, hash2)?;
        let binary = is_binary(&content1) || is_binary(&content2);
        let status = match (hash1, hash2) {
            (None, _) => "added",
            (_, None) => "deleted",
            _ => "modified",
        };
        let text1 = String::from_utf8_lossy(&content1);
        let text2 = String::from_utf8_lossy(&content2);

        if json {
            let (mut added, mut removed) = (0, 0);
            if !binary {
                for change in TextDiff::from_lines(&text1, &text2).iter_all_changes() {
                    match change.tag() {
                        ChangeTag::Insert => added += 1,
                        ChangeTag::Delete => removed += 1,
                        ChangeTag::Equal => {}
                    }
                }
            }
            changes.push(FileChange {
                path: path.clone(),
                status,
                binary,
                added,
                removed,
            });
        } else {
            println!("\n{}", format!("Diff for {} ({}):", path, status).bold());
            if binary {
                println!(
                    "Binary files differ ({} -> {} bytes)",
                    content1.len(),
                    content2.len()
                );
            } else {
                show_unified_diff(&text1, &text2, path);
            }
            changes.push(FileChange {
                path: path.clone(),
                status,
                binary,
                added: 0,
                removed: 0,
            });
        }
    }

    if json {
        println!(
            "{}",
            serde_json::json!({"v": 1, "from": from.id, "to": to.id, "files": changes})
        );
    } else if changes.is_empty() {
        if file_path_opt.is_some() {
            println!("Files are identical.");
        } else {
            println!("No differences.");
        }
    }
    Ok(())
}

fn read_optional(root: &Path, hash: Option<&String>) -> Result<Vec<u8>> {
    match hash {
        Some(hash) => read_content_from_objects(root, hash),
        None => Ok(Vec::new()),
    }
}

/// Git's heuristic: a NUL byte in the first 8 KB.
fn is_binary(content: &[u8]) -> bool {
    content.iter().take(8000).any(|&b| b == 0)
}
