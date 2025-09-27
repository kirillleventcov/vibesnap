use crate::error::Result;
use crate::vibe::{
    objects::read_content_from_objects,
    snapshot::{SnapshotManifest, build_snapshot_manifest},
};
use colored::*;
use indicatif::{ProgressBar, ProgressIterator, ProgressStyle};
use std::fs;
use std::path::{Path, PathBuf};

pub fn build_snapshot_manifest_with_progress(
    root: &Path,
    paths: &[PathBuf],
) -> Result<SnapshotManifest> {
    println!("{}", "Building snapshot...".cyan());
    let progress_bar = ProgressBar::new_spinner();
    progress_bar.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner:.green} [{elapsed_precise}] {msg}")
            .unwrap(),
    );
    progress_bar.set_message("Scanning files...");

    let manifest = build_snapshot_manifest(root, paths)?;

    progress_bar.finish_with_message("Snapshot manifest built.");
    Ok(manifest)
}

pub fn restore_files_from_manifest_with_progress(
    root: &Path,
    manifest: &SnapshotManifest,
) -> Result<()> {
    let total_files = manifest.files.len() as u64;
    let progress_bar = ProgressBar::new(total_files);
    progress_bar.set_style(
        ProgressStyle::default_bar()
            .template(
                "{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({percent}%)",
            )
            .unwrap()
            .progress_chars("#>-"),
    );

    for (file_path, hash) in manifest.files.iter().progress_with(progress_bar) {
        let dest_path = root.join(file_path);
        if let Some(parent_dir) = dest_path.parent() {
            fs::create_dir_all(parent_dir)?;
        }
        let content = read_content_from_objects(root, hash)?;
        fs::write(&dest_path, content)?;
    }

    Ok(())
}
