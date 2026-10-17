use std::fs::{self, File, Permissions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

fn tmp_path(dir: &Path) -> PathBuf {
    let n = TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    dir.join(format!(".vibesnap-tmp-{}-{}", std::process::id(), n))
}

/// Writes `content` to a temporary file in `tmp_dir` and renames it over `dest`, so
/// a reader (or a crash) never sees a half-written file. `tmp_dir` must be on the
/// same filesystem as `dest`. Renaming also replaces a symlink at `dest` instead of
/// writing through it.
pub fn write_atomic(
    tmp_dir: &Path,
    dest: &Path,
    content: &[u8],
    permissions: Option<Permissions>,
) -> std::io::Result<()> {
    let tmp = tmp_path(tmp_dir);
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
