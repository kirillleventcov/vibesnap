use crate::error::Result;
use std::fs::{File, OpenOptions};
use std::path::Path;

use super::constants::{LOCK_FILENAME, REPO_DIRNAME};

/// Exclusive writer lock on `.vibe/lock`. Every command that changes the store or
/// the working tree holds it for its whole read-modify-write sequence, so parallel
/// snaps cannot read the same HEAD and lose the parent chain. Readers never take it.
/// The lock is released when the guard is dropped (the file is closed).
pub struct RepoLock {
    _file: File,
}

pub fn lock_repo(root: &Path) -> Result<RepoLock> {
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(root.join(REPO_DIRNAME).join(LOCK_FILENAME))?;
    file.lock()?;
    Ok(RepoLock { _file: file })
}
