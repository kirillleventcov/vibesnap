use thiserror::Error;

#[derive(Error, Debug)]
pub enum AppError {
    #[error("Not inside a VibeSnap repo. Run 'vibesnap init' first.")]
    NotInRepo,
    #[error(".vibe already exists in this directory")]
    RepoExists,
    #[error("Database error: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Object not found: {0}")]
    ObjectNotFound(String),
    #[error("Checkpoint not found: {0}")]
    CheckpointNotFound(String),
    #[error("Track already exists: {0}")]
    TrackExists(String),
    #[error("Track not found: {0}")]
    TrackNotFound(String),
    #[error("Need at least two checkpoints to diff")]
    NotEnoughCheckpointsForDiff,
    #[error("Interactive prompt error: {0}")]
    Dialoguer(#[from] dialoguer::Error),
    /// Bad or missing arguments; exits with code 2 like clap's own usage errors.
    #[error("{0}")]
    Usage(String),
    #[error("{0}")]
    Generic(String),
}

impl AppError {
    pub fn exit_code(&self) -> i32 {
        match self {
            AppError::Usage(_) => 2,
            _ => 1,
        }
    }

    pub fn kind(&self) -> &'static str {
        match self {
            AppError::NotInRepo => "not_in_repo",
            AppError::RepoExists => "repo_exists",
            AppError::Db(_) => "db",
            AppError::Io(_) => "io",
            AppError::ObjectNotFound(_) => "object_not_found",
            AppError::CheckpointNotFound(_) => "checkpoint_not_found",
            AppError::TrackExists(_) => "track_exists",
            AppError::TrackNotFound(_) => "track_not_found",
            AppError::NotEnoughCheckpointsForDiff => "not_enough_checkpoints",
            AppError::Dialoguer(_) => "prompt",
            AppError::Usage(_) => "usage",
            AppError::Generic(_) => "error",
        }
    }
}

pub type Result<T> = std::result::Result<T, AppError>;
