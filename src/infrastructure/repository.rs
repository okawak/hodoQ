//! SQLite connection lifecycle and transactions spanning multiple entity types.
mod catalogs;
mod mapping;
mod tasks;
#[cfg(test)]
mod tests;
mod transfer;

use super::{RepositoryError, migrations};
use crate::domain::{Project, ProjectId, Tag, TagId, Task};
use catalogs::{save_project_on_connection, save_tag_on_connection};
use rusqlite::{Connection, OpenFlags};
use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};
use tasks::save_task_in_transaction;
use time::OffsetDateTime;

pub struct SqliteRepository {
    connection: Connection,
    path: Option<PathBuf>,
}

impl SqliteRepository {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, RepositoryError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let connection = Connection::open(path)?;
        Self::initialize(connection, Some(path.to_path_buf()))
    }

    pub fn open_in_memory() -> Result<Self, RepositoryError> {
        Self::initialize(Connection::open_in_memory()?, None)
    }

    pub fn open_read_only(path: impl AsRef<Path>) -> Result<Self, RepositoryError> {
        let path = path.as_ref();
        let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        connection.busy_timeout(Duration::from_secs(3))?;
        Ok(Self {
            connection,
            path: Some(path.to_path_buf()),
        })
    }

    fn initialize(
        mut connection: Connection,
        path: Option<PathBuf>,
    ) -> Result<Self, RepositoryError> {
        connection.pragma_update(None, "foreign_keys", "ON")?;
        connection.pragma_update(None, "journal_mode", "DELETE")?;
        connection.busy_timeout(Duration::from_secs(3))?;
        if let Some(database_path) = path.as_deref() {
            let version: i64 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
            let object_count: i64 = connection.query_row(
                "SELECT count(*) FROM sqlite_master WHERE type IN ('table', 'index')",
                [],
                |row| row.get(0),
            )?;
            if version < migrations::CURRENT_SCHEMA_VERSION && object_count > 0 {
                let backup_directory = database_path
                    .parent()
                    .unwrap_or_else(|| Path::new("."))
                    .join("backups");
                fs::create_dir_all(&backup_directory)?;
                let destination = backup_directory.join(format!(
                    "hodoq-before-migration-{}.sqlite3",
                    OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000
                ));
                let mut destination_connection = Connection::open(destination)?;
                let backup =
                    rusqlite::backup::Backup::new(&connection, &mut destination_connection)?;
                backup.run_to_completion(16, Duration::from_millis(20), None)?;
            }
        }
        migrations::migrate(&mut connection)?;
        Ok(Self { connection, path })
    }

    pub fn database_path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub fn apply_history_state(
        &mut self,
        tasks: Option<&[Task]>,
        projects_to_save: &[Project],
        projects_to_delete: &[ProjectId],
        tags_to_save: &[Tag],
        tags_to_delete: &[TagId],
    ) -> Result<(), RepositoryError> {
        let transaction = self.connection.transaction()?;
        for project in projects_to_save {
            save_project_on_connection(&transaction, project)?;
        }
        for tag in tags_to_save {
            save_tag_on_connection(&transaction, tag)?;
        }
        if let Some(tasks) = tasks {
            transaction.execute("DELETE FROM tasks", [])?;
            for task in tasks {
                save_task_in_transaction(&transaction, task)?;
            }
        }
        for id in projects_to_delete {
            transaction.execute("DELETE FROM projects WHERE id = ?1", [id.to_string()])?;
        }
        for id in tags_to_delete {
            transaction.execute("DELETE FROM tags WHERE id = ?1", [id.to_string()])?;
        }
        transaction.commit()?;
        Ok(())
    }
}
