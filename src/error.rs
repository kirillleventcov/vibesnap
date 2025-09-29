use thiserror::Error;

#[derive(Error, Debug)]
pub enum AppError {
    #[error("Not inside a VibeSnap repo. Run 'vibesnap init' first.")]
    NotInRepo,
    #[error(".vibe already exists in this directory")]
    RepoExists,
    #[error("SQLite error: {0}")]
    DbError(#[from] rusqlite::Error),
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
    #[error("Invalid HEAD file format")]
    InvalidHead,
    #[error("Object not found in store: {0}")]
    ObjectNotFound(String),
    #[error("Snapshot manifest not found for {0}")]
    ManifestNotFound(String),
    #[error("Could not serialize manifest: {0}")]
    ManifestSerializationError(String),
    #[error("Could not deserialize manifest: {0}")]
    ManifestDeserializationError(String),
    #[error("Track already exists: {0}")]
    TrackExists(String),
    #[error("Track not found in DB: {0}")]
    TrackNotFound(String),
    #[error("Need at least two checkpoints to diff")]
    NotEnoughCheckpointsForDiff,
    #[error("Dialoguer error: {0}")]
    DialoguerError(#[from] dialoguer::Error),
    #[error("{0}")]
    Generic(String),
}

pub type Result<T> = std::result::Result<T, AppError>;
