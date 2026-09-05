mod database_worker;
mod error;
mod instance_lock;
mod migrations;
mod paths;
mod repository;
mod settings;

pub use database_worker::DatabaseWorker;
// Preserve the existing public path while keeping the read model independent of storage.
pub use crate::domain::AppDataSnapshot;
pub use error::RepositoryError;
pub use instance_lock::InstanceLock;
pub use paths::AppPaths;
pub use repository::SqliteRepository;
pub use settings::{AppSettings, WindowSettings};
