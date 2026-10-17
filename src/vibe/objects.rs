use crate::error::{AppError, Result};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

use super::constants::{OBJECTS_DIRNAME, REPO_DIRNAME, TMP_DIRNAME};
use super::fsutil::write_atomic;

fn hash_file_content(content: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content);
    hex::encode(hasher.finalize())
}

/// Content-addressed object store under `.vibe/objects`.
pub struct ObjectStore {
    objects_dir: PathBuf,
    tmp_dir: PathBuf,
}

impl ObjectStore {
    pub fn open(root: &Path) -> Result<ObjectStore> {
        let vibe_dir = root.join(REPO_DIRNAME);
        let store = ObjectStore {
            objects_dir: vibe_dir.join(OBJECTS_DIRNAME),
            tmp_dir: vibe_dir.join(TMP_DIRNAME),
        };
        fs::create_dir_all(&store.objects_dir)?;
        fs::create_dir_all(&store.tmp_dir)?;
        Ok(store)
    }

    pub fn store(&self, content: &[u8]) -> Result<String> {
        let hash = hash_file_content(content);
        let object_path = self.objects_dir.join(&hash);

        // Objects are written by temp file and rename, so an existing object is
        // complete. The size check also repairs objects truncated by older versions,
        // which wrote in place.
        if let Ok(meta) = fs::metadata(&object_path)
            && meta.len() == content.len() as u64
        {
            return Ok(hash);
        }
        write_atomic(&self.tmp_dir, &object_path, content, None)?;
        Ok(hash)
    }
}

pub fn read_content_from_objects(root: &Path, hash: &str) -> Result<Vec<u8>> {
    let object_path = root.join(REPO_DIRNAME).join(OBJECTS_DIRNAME).join(hash);

    let content = match fs::read(&object_path) {
        Ok(content) => content,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(AppError::ObjectNotFound(hash.to_string()));
        }
        Err(e) => return Err(AppError::Io(e)),
    };
    if hash_file_content(&content) != hash {
        return Err(AppError::Generic(format!(
            "Object {hash} is corrupt (its content does not match its hash)"
        )));
    }
    Ok(content)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncated_object_is_rewritten() {
        let dir = tempfile::tempdir().unwrap();
        let store = ObjectStore::open(dir.path()).unwrap();
        let content = b"hello world, this is the full content";
        let hash = hash_file_content(content);
        let object_path = dir.path().join(".vibe/objects").join(&hash);
        fs::write(&object_path, &content[..5]).unwrap();

        assert!(read_content_from_objects(dir.path(), &hash).is_err());
        assert_eq!(store.store(content).unwrap(), hash);
        assert_eq!(
            read_content_from_objects(dir.path(), &hash).unwrap(),
            content
        );
        // No temp files left behind.
        assert_eq!(
            fs::read_dir(dir.path().join(".vibe/tmp")).unwrap().count(),
            0
        );
    }
}
