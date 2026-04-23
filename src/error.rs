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
    #[error("{0}")]
    Generic(String),
}

pub type Result<T> = std::result::Result<T, AppError>;
