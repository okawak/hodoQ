//! Backup/restore and stable CSV/JSON interchange formats.
use super::{RepositoryError, SqliteRepository, migrations};
use crate::domain::{Due, Project, SavedView, Tag, Task, TaskFilter};
use rusqlite::Connection;
use serde::Serialize;
use std::{
    fs::{self, File},
    io::Write,
    path::Path,
    time::Duration,
};
use time::{OffsetDateTime, format_description::well_known::Iso8601};

impl SqliteRepository {
    pub fn create_backup(&self, destination: &Path) -> Result<(), RepositoryError> {
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut destination_connection = Connection::open(destination)?;
        let backup = rusqlite::backup::Backup::new(&self.connection, &mut destination_connection)?;
        backup.run_to_completion(16, Duration::from_millis(20), None)?;
        Ok(())
    }

    pub fn integrity_check(path: &Path) -> Result<bool, RepositoryError> {
        let connection =
            Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let result: String =
            connection.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
        Ok(result == "ok")
    }

    pub fn restore_from_backup(
        &mut self,
        source: &Path,
        safety_backup: &Path,
    ) -> Result<(), RepositoryError> {
        let candidate = Self::validated_backup(source).map_err(|error| match error {
            RepositoryError::NewerSchema { .. } | RepositoryError::InvalidBackup(_) => error,
            error => RepositoryError::InvalidBackup(error.to_string()),
        })?;
        self.create_backup(safety_backup)?;
        // Backup commits the validated image atomically. Do not run fallible
        // migrations or decode data after replacing the live database.
        let backup = rusqlite::backup::Backup::new(&candidate.connection, &mut self.connection)?;
        backup.run_to_completion(16, Duration::from_millis(20), None)?;
        Ok(())
    }

    fn validated_backup(source: &Path) -> Result<Self, RepositoryError> {
        let source_connection =
            Connection::open_with_flags(source, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let mut connection = Connection::open_in_memory()?;
        {
            let backup = rusqlite::backup::Backup::new(&source_connection, &mut connection)?;
            backup.run_to_completion(16, Duration::from_millis(20), None)?;
        }
        let version: i64 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if version > migrations::CURRENT_SCHEMA_VERSION {
            return Err(RepositoryError::NewerSchema {
                found: version,
                supported: migrations::CURRENT_SCHEMA_VERSION,
            });
        }
        if version < 1 {
            return Err(RepositoryError::InvalidBackup(
                "not a versioned HodoQ backup".to_owned(),
            ));
        }
        let integrity: String =
            connection.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
        if integrity != "ok" {
            return Err(RepositoryError::InvalidBackup(
                "integrity_check failed".to_owned(),
            ));
        }
        connection.pragma_update(None, "foreign_keys", "ON")?;
        migrations::migrate(&mut connection)?;
        let candidate = Self {
            connection,
            path: None,
        };
        // Decode every entity before touching the current database. SQLite's
        // integrity_check alone accepts unrelated schemas and invalid JSON/IDs.
        candidate.list_all_tasks()?;
        candidate.list_projects()?;
        candidate.list_tags()?;
        candidate.list_views()?;
        let mut statement = candidate.connection.prepare("PRAGMA foreign_key_check")?;
        if statement.query([])?.next()?.is_some() {
            return Err(RepositoryError::InvalidBackup(
                "foreign_key_check failed".to_owned(),
            ));
        }
        drop(statement);
        Ok(candidate)
    }

    pub fn export_csv(
        &self,
        destination: &Path,
        filter: &TaskFilter,
        with_bom: bool,
    ) -> Result<(), RepositoryError> {
        let tasks = self.list_tasks(filter, &[])?;
        Self::export_tasks_csv(destination, &tasks, with_bom)
    }

    pub fn export_tasks_csv(
        destination: &Path,
        tasks: &[Task],
        with_bom: bool,
    ) -> Result<(), RepositoryError> {
        let mut file = File::create(destination)?;
        if with_bom {
            file.write_all(&[0xEF, 0xBB, 0xBF])?;
        }
        let mut writer = csv::Writer::from_writer(file);
        writer.write_record([
            "id",
            "title",
            "memo",
            "status",
            "priority",
            "progress",
            "due",
            "project_id",
            "tag_ids",
            "created_at",
            "updated_at",
        ])?;
        for task in tasks {
            writer.write_record([
                task.id.to_string(),
                task.title.clone(),
                task.memo.clone(),
                task.status.as_str().to_owned(),
                task.priority.as_str().to_owned(),
                task.progress.to_string(),
                due_display_value(&task.due)?,
                task.project_id.map(|id| id.to_string()).unwrap_or_default(),
                task.tag_ids
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(";"),
                task.created_at.format(&Iso8601::DEFAULT)?,
                task.updated_at.format(&Iso8601::DEFAULT)?,
            ])?;
        }
        writer.flush()?;
        Ok(())
    }

    pub fn export_json(&self, destination: &Path) -> Result<(), RepositoryError> {
        let data = ExportData {
            format_version: 1,
            exported_at: OffsetDateTime::now_utc(),
            tasks: self.list_all_tasks()?,
            projects: self.list_projects()?,
            tags: self.list_tags()?,
            saved_views: self.list_views()?,
        };
        fs::write(destination, serde_json::to_vec_pretty(&data)?)?;
        Ok(())
    }
}

fn due_display_value(due: &Due) -> Result<String, RepositoryError> {
    match due {
        Due::None => Ok(String::new()),
        Due::Date(date) => Ok(date.format(&Iso8601::DATE)?),
        Due::DateTime(date_time) => Ok(date_time.format(&Iso8601::DEFAULT)?),
    }
}

#[derive(Serialize)]
struct ExportData {
    format_version: u32,
    exported_at: OffsetDateTime,
    tasks: Vec<Task>,
    projects: Vec<Project>,
    tags: Vec<Tag>,
    saved_views: Vec<SavedView>,
}
