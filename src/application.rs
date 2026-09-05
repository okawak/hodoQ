use std::path::{Path, PathBuf};

use time::OffsetDateTime;

use crate::{
    domain::{Project, ProjectId, SavedView, SavedViewId, Tag, TagId, Task, TaskId},
    infrastructure::{DatabaseWorker, RepositoryError},
};

pub(crate) use crate::domain::AppDataSnapshot;

/// An application failure with its original storage error preserved for diagnostics.
/// The concrete error is private so presentation only relies on Display/Error.
#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub(crate) struct ApplicationError(#[from] RepositoryError);

impl From<std::io::Error> for ApplicationError {
    fn from(error: std::io::Error) -> Self {
        Self(RepositoryError::from(error))
    }
}

/// Application boundary used by the presentation layer.
///
/// It keeps the GUI independent from the concrete SQLite worker while retaining
/// ordered, acknowledged writes on the dedicated database thread.
#[derive(Clone)]
pub(crate) struct TaskApplication {
    database: DatabaseWorker,
}

impl TaskApplication {
    pub(crate) fn start(path: &Path) -> Result<Self, ApplicationError> {
        Ok(Self {
            database: DatabaseWorker::start(path)?,
        })
    }

    /// Reconnect only when writes can resume; keep the existing workspace on failure.
    pub(crate) fn reconnect(path: &Path) -> Result<(Self, AppDataSnapshot), ApplicationError> {
        let application = Self::start(path)?;
        let snapshot = application.load()?;
        if application.is_read_only() {
            return Err(RepositoryError::ReadOnly.into());
        }
        Ok((application, snapshot))
    }

    pub(crate) fn is_read_only(&self) -> bool {
        self.database.is_read_only()
    }

    pub(crate) fn startup_warning(&self) -> Option<&str> {
        self.database.startup_warning()
    }

    pub(crate) fn load(&self) -> Result<AppDataSnapshot, ApplicationError> {
        self.database.load().map_err(Into::into)
    }

    pub(crate) fn save_task(&self, task: Task) -> Result<(), ApplicationError> {
        self.database.save_task(task).map_err(Into::into)
    }

    pub(crate) fn save_tasks(&self, tasks: Vec<Task>) -> Result<(), ApplicationError> {
        self.database.save_tasks(tasks).map_err(Into::into)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn apply_history_state(
        &self,
        tasks: Option<Vec<Task>>,
        projects_to_save: Vec<Project>,
        projects_to_delete: Vec<ProjectId>,
        tags_to_save: Vec<Tag>,
        tags_to_delete: Vec<TagId>,
    ) -> Result<(), ApplicationError> {
        self.database
            .apply_history_state(
                tasks,
                projects_to_save,
                projects_to_delete,
                tags_to_save,
                tags_to_delete,
            )
            .map_err(Into::into)
    }

    pub(crate) fn move_task_to_trash(
        &self,
        id: TaskId,
        now: OffsetDateTime,
    ) -> Result<(), ApplicationError> {
        self.database
            .move_task_to_trash(id, now)
            .map_err(Into::into)
    }

    pub(crate) fn restore_task(
        &self,
        id: TaskId,
        now: OffsetDateTime,
    ) -> Result<(), ApplicationError> {
        self.database.restore_task(id, now).map_err(Into::into)
    }

    pub(crate) fn save_view(&self, view: SavedView) -> Result<(), ApplicationError> {
        self.database.save_view(view).map_err(Into::into)
    }

    pub(crate) fn delete_view(&self, id: SavedViewId) -> Result<(), ApplicationError> {
        self.database.delete_view(id).map_err(Into::into)
    }

    pub(crate) fn purge_expired_trash(&self, now: OffsetDateTime) -> Result<(), ApplicationError> {
        self.database.purge_expired_trash(now).map_err(Into::into)
    }

    pub(crate) fn empty_trash(&self) -> Result<(), ApplicationError> {
        self.database.empty_trash().map_err(Into::into)
    }

    pub(crate) fn create_backup(&self, destination: PathBuf) -> Result<(), ApplicationError> {
        self.database.create_backup(destination).map_err(Into::into)
    }

    pub(crate) fn restore_backup(
        &self,
        source: PathBuf,
        safety_backup: PathBuf,
    ) -> Result<AppDataSnapshot, ApplicationError> {
        self.database
            .restore_backup(source, safety_backup)
            .map_err(Into::into)
    }

    pub(crate) fn export_task_csv(
        &self,
        destination: PathBuf,
        tasks: Vec<Task>,
        with_bom: bool,
    ) -> Result<(), ApplicationError> {
        self.database
            .export_task_csv(destination, tasks, with_bom)
            .map_err(Into::into)
    }

    pub(crate) fn export_json(&self, destination: PathBuf) -> Result<(), ApplicationError> {
        self.database.export_json(destination).map_err(Into::into)
    }

    pub(crate) fn take_error(&self) -> Option<String> {
        self.database.take_error()
    }

    pub(crate) fn flush(&self) -> Result<(), ApplicationError> {
        self.database.flush().map_err(Into::into)
    }
}

/// One reversible application-level operation.
///
/// A single entry may span tasks and their related project or tag so that
/// relationship changes are restored together.
#[derive(Debug, Clone)]
pub(crate) struct HistoryEntry {
    pub(crate) task_changes: Vec<(Option<Task>, Option<Task>)>,
    pub(crate) project_changes: Vec<(Option<Project>, Option<Project>)>,
    pub(crate) tag_changes: Vec<(Option<Tag>, Option<Tag>)>,
}

impl HistoryEntry {
    pub(crate) fn tasks(changes: Vec<(Option<Task>, Option<Task>)>) -> Self {
        Self {
            task_changes: changes,
            project_changes: Vec::new(),
            tag_changes: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reconnect_returns_the_committed_snapshot() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("tasks.sqlite3");
        let application = TaskApplication::start(&path).unwrap();
        let task = Task::new("persisted", OffsetDateTime::UNIX_EPOCH).unwrap();
        application.save_task(task.clone()).unwrap();
        application.flush().unwrap();
        let (reconnected, snapshot) = TaskApplication::reconnect(&path).unwrap();
        assert!(!reconnected.is_read_only());
        assert_eq!(snapshot.tasks, vec![task]);
    }

    #[test]
    fn reconnect_rejects_read_only_recovery_without_modifying_data() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("tasks.sqlite3");
        let mut repository = crate::infrastructure::SqliteRepository::open(&path).unwrap();
        let task = Task::new("preserved", OffsetDateTime::UNIX_EPOCH).unwrap();
        repository.save_task(&task).unwrap();
        drop(repository);
        let connection = rusqlite::Connection::open(&path).unwrap();
        connection.pragma_update(None, "user_version", 999).unwrap();
        drop(connection);
        assert!(matches!(
            TaskApplication::reconnect(&path),
            Err(ApplicationError(RepositoryError::ReadOnly))
        ));
        let repository = crate::infrastructure::SqliteRepository::open_read_only(&path).unwrap();
        assert_eq!(repository.task(task.id).unwrap(), Some(task));
    }
}
