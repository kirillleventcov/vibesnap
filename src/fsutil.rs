use std::fs::{self, File, Permissions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

pub fn tmp_path(dir: &Path, prefix: &str) -> PathBuf {
    let n = TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    dir.join(format!("{prefix}{}-{}", std::process::id(), n))
}

/// Writes `content` to a temporary file next to `dest` and renames it over
/// `dest`, so a reader (or a crash) never sees a half-written file. Renaming
/// also replaces a symlink at `dest` instead of writing through it.
pub fn write_atomic(
    dest: &Path,
    content: &[u8],
    permissions: Option<Permissions>,
) -> std::io::Result<()> {
    let dir = dest.parent().unwrap_or(Path::new("."));
    let tmp = tmp_path(dir, ".vibesnap-tmp-");
    let result = (|| {
        let mut file = File::create_new(&tmp)?;
        file.write_all(content)?;
        if let Some(permissions) = permissions {
            file.set_permissions(permissions)?;
        }
        drop(file);
        fs::rename(&tmp, dest)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

/// Creates (or replaces) a symlink at `dest` atomically.
pub fn symlink_atomic(target: &Path, dest: &Path) -> std::io::Result<()> {
    let dir = dest.parent().unwrap_or(Path::new("."));
    let tmp = tmp_path(dir, ".vibesnap-tmp-");
    std::os::unix::fs::symlink(target, &tmp)?;
    let result = fs::rename(&tmp, dest);
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

/// Removes now-empty directories from `file`'s parent up to (not including) `root`.
pub fn remove_empty_parents(root: &Path, file: &Path) {
    let mut dir = file.parent();
    while let Some(d) = dir {
        if d == root || !d.starts_with(root) || fs::remove_dir(d).is_err() {
            break;
        }
        dir = d.parent();
    }
}

pub fn now_ns() -> i64 {
    chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
}
