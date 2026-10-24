use thiserror::Error;

#[derive(Error, Debug)]
pub enum AppError {
    #[error("Not inside a vibesnap store. Run 'vs init' first.")]
    NotInRepo,
    #[error("Database error: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Git object store error: {0}")]
    Git(String),
    /// Bad or missing arguments; exits with code 2 like clap's own usage errors.
    #[error("{0}")]
    Usage(String),
    /// Refused because applying the change would conflict; exits with code 3.
    #[error("{0}")]
    Conflict(String),
    #[error("{0}")]
    Msg(String),
}

impl AppError {
    pub fn exit_code(&self) -> i32 {
        match self {
            AppError::Usage(_) => 2,
            AppError::Conflict(_) => 3,
            _ => 1,
        }
    }

    pub fn kind(&self) -> &'static str {
        match self {
            AppError::NotInRepo => "not_in_repo",
            AppError::Db(_) => "db",
            AppError::Io(_) => "io",
            AppError::Git(_) => "git",
            AppError::Usage(_) => "usage",
            AppError::Conflict(_) => "conflict",
            AppError::Msg(_) => "error",
        }
    }
}

impl From<rusqlite::types::FromSqlError> for AppError {
    fn from(e: rusqlite::types::FromSqlError) -> Self {
        AppError::Db(rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Null,
            Box::new(e),
        ))
    }
}

pub fn git_err(e: impl std::fmt::Display) -> AppError {
    AppError::Git(e.to_string())
}

pub fn msg(text: impl Into<String>) -> AppError {
    AppError::Msg(text.into())
}

pub type Result<T> = std::result::Result<T, AppError>;
