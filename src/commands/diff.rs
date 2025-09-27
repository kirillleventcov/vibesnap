use crate::cli::display::{show_side_by_side_diff, show_unified_diff};
use crate::error::Result;
use crate::vibe::{
    objects::read_content_from_objects, repo::find_repo_root, snapshot::load_snapshot_manifest,
};
use colored::*;
use std::collections::HashSet;
use std::path::PathBuf;

pub fn diff_command(
    id1: String,
    id2: String,
    file_path_opt: Option<PathBuf>,
    side_by_side: bool,
) -> Result<()> {
    let root = find_repo_root(None)?;
    let manifest1 = load_snapshot_manifest(&root, &id1)?;
    let manifest2 = load_snapshot_manifest(&root, &id2)?;

    if let Some(relative_file_path) = file_path_opt {
        let path_str = relative_file_path.to_string_lossy();
        let text1 = manifest1
            .files
            .get(path_str.as_ref())
            .map(|hash| read_content_from_objects(&root, hash).unwrap_or_default())
            .map(|bytes| String::from_utf8_lossy(&bytes).to_string())
            .unwrap_or_default();
        let text2 = manifest2
            .files
            .get(path_str.as_ref())
            .map(|hash| read_content_from_objects(&root, hash).unwrap_or_default())
            .map(|bytes| String::from_utf8_lossy(&bytes).to_string())
            .unwrap_or_default();

        if text1 == text2 {
            println!("Files are identical.");
            return Ok(());
        }

        if side_by_side {
            show_side_by_side_diff(&text1, &text2, &path_str);
        } else {
            show_unified_diff(&text1, &text2, &path_str);
        }
    } else {
        let files1: HashSet<_> = manifest1.files.keys().collect();
        let files2: HashSet<_> = manifest2.files.keys().collect();
        let all_files: Vec<_> = files1.union(&files2).collect();

        for file_path in all_files {
            let hash1 = manifest1.files.get(*file_path);
            let hash2 = manifest2.files.get(*file_path);

            if hash1 == hash2 {
                continue;
            }

            println!("\n{}", format!("Diff for {}:", file_path).bold());

            let text1 = hash1
                .map(|hash| read_content_from_objects(&root, hash).unwrap_or_default())
                .map(|bytes| String::from_utf8_lossy(&bytes).to_string())
                .unwrap_or_default();
            let text2 = hash2
                .map(|hash| read_content_from_objects(&root, hash).unwrap_or_default())
                .map(|bytes| String::from_utf8_lossy(&bytes).to_string())
                .unwrap_or_default();

            if side_by_side {
                show_side_by_side_diff(&text1, &text2, file_path);
            } else {
                show_unified_diff(&text1, &text2, file_path);
            }
        }
    }

    Ok(())
}
