use crate::error::{AppError, Result};
use std::fs;
use std::path::{Path, PathBuf};

use super::constants::{DEFAULT_TRACK, HEAD_FILENAME, REPO_DIRNAME};

pub fn find_repo_root(start: Option<PathBuf>) -> Result<PathBuf> {
    let current_dir: PathBuf =
        start.unwrap_or_else(|| std::env::current_dir().expect("Failed to get current directory"));
    let mut current: &Path = current_dir.as_path();

    loop {
        if current.join(REPO_DIRNAME).is_dir() {
            return Ok(current.to_path_buf());
        }
        match current.parent() {
            Some(parent) => current = parent,
            None => return Err(AppError::NotInRepo),
        }
    }
}

pub fn read_head(root: &Path) -> Result<(String, Option<String>)> {
    let head_file: PathBuf = root.join(REPO_DIRNAME).join(HEAD_FILENAME);
    if !head_file.exists() {
        // If HEAD doesn't exist, initialize with default track and no checkpoint ID
        // This situation might occur if init was incomplete or HEAD was deleted.
        // We'll also write this default state to HEAD to ensure consistency.
        write_head(root, DEFAULT_TRACK, None)?;
        return Ok((DEFAULT_TRACK.to_string(), None));
    }
    let content: String = fs::read_to_string(head_file)?;
    let parts: Vec<&str> = content.trim().split_whitespace().collect();
    match parts.as_slice() {
        [track] => Ok((track.to_string(), None)),
        [track, checkpoint_id] => Ok((track.to_string(), Some(checkpoint_id.to_string()))),
        _ => Err(AppError::InvalidHead),
    }
}

pub fn write_head(root: &Path, track: &str, checkpoint_id: Option<&str>) -> Result<()> {
    let head_file: PathBuf = root.join(REPO_DIRNAME).join(HEAD_FILENAME);
    let content: String = match checkpoint_id {
        Some(id) => format!("{} {}\n", track, id),
        None => format!("{}\n", track),
    };
    fs::write(head_file, content).map_err(AppError::IoError)
}
