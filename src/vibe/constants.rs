pub const REPO_DIRNAME: &str = ".vibe";
pub const DB_FILENAME: &str = "meta.db";
pub const OBJECTS_DIRNAME: &str = "objects";
pub const SNAPSHOTS_DIRNAME: &str = "snapshots";
pub const TMP_DIRNAME: &str = "tmp";
pub const LOCK_FILENAME: &str = "lock";
/// Legacy location of the HEAD pointer; it now lives in the database.
pub const HEAD_FILENAME: &str = "HEAD";
pub const DEFAULT_TRACK: &str = "main";

/// Top-level directories that are never snapshotted, restored into or deleted,
/// whatever the ignore files say.
pub const RESERVED_DIRS: &[&str] = &[REPO_DIRNAME, ".git"];
