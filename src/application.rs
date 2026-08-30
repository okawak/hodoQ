use std::path::{Path, PathBuf};

use time::OffsetDateTime;

use crate::{
    domain::{Project, ProjectId, SavedView, SavedViewId, Tag, TagId, Task, TaskId},
    infrastructure::{AppDataSnapshot, DatabaseWorker, RepositoryError},
};

/// Application boundary used by the presentation layer.
///
/// It keeps the GUI independent from the concrete SQLite worker while retaining
/// ordered, acknowledged writes on the dedicated database thread.
#[derive(Clone)]
pub(crate) struct TaskApplication {
    database: DatabaseWorker,
}

impl TaskApplication {
    pub(crate) fn start(path: &Path) -> Result<Self, RepositoryError> {
        Ok(Self {
            database: DatabaseWorker::start(path)?,
        })
    }

    pub(crate) fn is_read_only(&self) -> bool {
        self.database.is_read_only()
    }

    pub(crate) fn startup_warning(&self) -> Option<&str> {
        self.database.startup_warning()
    }

    pub(crate) fn load(&self) -> Result<AppDataSnapshot, RepositoryError> {
        self.database.load()
    }

    pub(crate) fn save_task(&self, task: Task) -> Result<(), RepositoryError> {
        self.database.save_task(task)
    }

    pub(crate) fn save_tasks(&self, tasks: Vec<Task>) -> Result<(), RepositoryError> {
        self.database.save_tasks(tasks)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn apply_history_state(
        &self,
        tasks: Option<Vec<Task>>,
        projects_to_save: Vec<Project>,
        projects_to_delete: Vec<ProjectId>,
        tags_to_save: Vec<Tag>,
        tags_to_delete: Vec<TagId>,
    ) -> Result<(), RepositoryError> {
        self.database.apply_history_state(
            tasks,
            projects_to_save,
            projects_to_delete,
            tags_to_save,
            tags_to_delete,
        )
    }

    pub(crate) fn move_task_to_trash(
        &self,
        id: TaskId,
        now: OffsetDateTime,
    ) -> Result<(), RepositoryError> {
        self.database.move_task_to_trash(id, now)
    }

    pub(crate) fn restore_task(
        &self,
        id: TaskId,
        now: OffsetDateTime,
    ) -> Result<(), RepositoryError> {
        self.database.restore_task(id, now)
    }

    pub(crate) fn save_project(&self, project: Project) -> Result<(), RepositoryError> {
        self.database.save_project(project)
    }

    pub(crate) fn save_projects(&self, projects: Vec<Project>) -> Result<(), RepositoryError> {
        self.database.save_projects(projects)
    }

    pub(crate) fn delete_project(&self, id: ProjectId) -> Result<(), RepositoryError> {
        self.database.delete_project(id)
    }

    pub(crate) fn save_tag(&self, tag: Tag) -> Result<(), RepositoryError> {
        self.database.save_tag(tag)
    }

    pub(crate) fn delete_tag(&self, id: TagId) -> Result<(), RepositoryError> {
        self.database.delete_tag(id)
    }

    pub(crate) fn save_view(&self, view: SavedView) -> Result<(), RepositoryError> {
        self.database.save_view(view)
    }

    pub(crate) fn delete_view(&self, id: SavedViewId) -> Result<(), RepositoryError> {
        self.database.delete_view(id)
    }

    pub(crate) fn purge_expired_trash(&self, now: OffsetDateTime) -> Result<(), RepositoryError> {
        self.database.purge_expired_trash(now)
    }

    pub(crate) fn empty_trash(&self) -> Result<(), RepositoryError> {
        self.database.empty_trash()
    }

    pub(crate) fn create_backup(&self, destination: PathBuf) -> Result<(), RepositoryError> {
        self.database.create_backup(destination)
    }

    pub(crate) fn restore_backup(
        &self,
        source: PathBuf,
        safety_backup: PathBuf,
    ) -> Result<AppDataSnapshot, RepositoryError> {
        self.database.restore_backup(source, safety_backup)
    }

    pub(crate) fn export_task_csv(
        &self,
        destination: PathBuf,
        tasks: Vec<Task>,
        with_bom: bool,
    ) -> Result<(), RepositoryError> {
        self.database.export_task_csv(destination, tasks, with_bom)
    }

    pub(crate) fn export_json(&self, destination: PathBuf) -> Result<(), RepositoryError> {
        self.database.export_json(destination)
    }

    pub(crate) fn take_error(&self) -> Option<String> {
        self.database.take_error()
    }

    pub(crate) fn flush(&self) -> Result<(), RepositoryError> {
        self.database.flush()
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
