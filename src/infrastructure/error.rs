use thiserror::Error;

#[derive(Debug, Error)]
pub enum RepositoryError {
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("file error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("CSV error: {0}")]
    Csv(#[from] csv::Error),
    #[error("date formatting error: {0}")]
    DateFormat(#[from] time::error::Format),
    #[error("application data directory is unavailable")]
    DataDirectoryUnavailable,
    #[error(
        "HodoQ repository root could not be found; run the executable inside the cloned repository or pass --data-dir"
    )]
    RepositoryRootUnavailable,
    #[error("HodoQ is already running with this data directory")]
    AlreadyRunning,
    #[error("database worker stopped")]
    WorkerStopped,
    #[error("database worker could not start: {0}")]
    WorkerInitialization(String),
    #[error("database operation failed: {0}")]
    WorkerOperation(String),
    #[error("database is open in read-only recovery mode")]
    ReadOnly,
    #[error("database schema {found} is newer than supported schema {supported}")]
    NewerSchema { found: i64, supported: i64 },
    #[error("invalid backup: {0}")]
    InvalidBackup(String),
}
