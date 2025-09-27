use crate::error::{AppError, Result};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

use super::constants::{OBJECTS_DIRNAME, REPO_DIRNAME};

fn hash_file_content(content: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content);
    hex::encode(hasher.finalize())
}

pub fn store_content_in_objects(root: &Path, content: &[u8]) -> Result<String> {
    let hash: String = hash_file_content(content);
    let objects_dir: PathBuf = root.join(REPO_DIRNAME).join(OBJECTS_DIRNAME);
    fs::create_dir_all(&objects_dir)?;

    let object_path = objects_dir.join(&hash);

    // Only write if object doesn't already exist (deduplication)
    if !object_path.exists() {
        fs::write(object_path, content)?;
    }

    Ok(hash)
}

pub fn read_content_from_objects(root: &Path, hash: &str) -> Result<Vec<u8>> {
    let object_path = root.join(REPO_DIRNAME).join(OBJECTS_DIRNAME).join(hash);

    if !object_path.exists() {
        return Err(AppError::ObjectNotFound(hash.to_string()));
    }

    fs::read(object_path).map_err(AppError::IoError)
}
