use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    thread,
};

use crossbeam_channel::{Receiver, Sender, bounded, unbounded};
use time::OffsetDateTime;

use crate::domain::{
    Project, ProjectId, SavedView, SavedViewId, Tag, TagId, Task, TaskFilter, TaskId,
};

use super::{RepositoryError, SqliteRepository};
use crate::domain::AppDataSnapshot;

enum DatabaseCommand {
    Load(Sender<Result<AppDataSnapshot, String>>),
    Execute {
        request_id: u64,
        operation: Box<DatabaseOperation>,
        response: Sender<DatabaseResult>,
    },
    RestoreBackup(PathBuf, PathBuf, Sender<Result<AppDataSnapshot, String>>),
    Flush(Sender<()>),
}

enum DatabaseOperation {
    SaveTask(Task),
    SaveTasks(Vec<Task>),
    ReplaceAllTasks(Vec<Task>),
    MoveTaskToTrash(TaskId, OffsetDateTime),
    RestoreTask(TaskId, OffsetDateTime),
    SaveProject(Project),
    SaveProjects(Vec<Project>),
    ApplyHistoryState {
        tasks: Option<Vec<Task>>,
        projects_to_save: Vec<Project>,
        projects_to_delete: Vec<ProjectId>,
        tags_to_save: Vec<Tag>,
        tags_to_delete: Vec<TagId>,
    },
    DeleteProject(ProjectId),
    SaveTag(Tag),
    DeleteTag(TagId),
    SaveView(SavedView),
    DeleteView(SavedViewId),
    PurgeExpiredTrash(OffsetDateTime),
    EmptyTrash,
    CreateBackup(PathBuf),
    ExportCsv(PathBuf, TaskFilter, bool),
    ExportTaskCsv(PathBuf, Vec<Task>, bool),
    ExportJson(PathBuf),
}

struct DatabaseResult {
    request_id: u64,
    result: Result<(), String>,
}

#[derive(Clone)]
pub struct DatabaseWorker {
    commands: Sender<DatabaseCommand>,
    errors: Receiver<String>,
    next_request_id: Arc<AtomicU64>,
    read_only: bool,
    startup_warning: Option<String>,
}

impl DatabaseWorker {
    pub fn start(path: &Path) -> Result<Self, RepositoryError> {
        let (commands_tx, commands_rx) = unbounded();
        let (errors_tx, errors_rx) = unbounded();
        let (ready_tx, ready_rx) = bounded(1);
        let path = path.to_path_buf();
        thread::Builder::new()
            .name("hodoq-database".to_owned())
            .spawn(move || match SqliteRepository::open(&path) {
                Ok(repository) => {
                    let _ = ready_tx.send(Ok((false, None)));
                    run_worker(repository, commands_rx, errors_tx);
                }
                Err(write_error) if path.exists() => {
                    match SqliteRepository::open_read_only(&path) {
                        Ok(repository) => {
                            let warning = format!(
                                "通常モードでDBを開けなかったため読み取り専用です: {write_error}"
                            );
                            let _ = ready_tx.send(Ok((true, Some(warning))));
                            run_worker(repository, commands_rx, errors_tx);
                        }
                        Err(_) => {
                            let message = write_error.to_string();
                            let _ = ready_tx.send(Err(message.clone()));
                            let _ = errors_tx.send(message);
                        }
                    }
                }
                Err(error) => {
                    let message = error.to_string();
                    let _ = ready_tx.send(Err(message.clone()));
                    let _ = errors_tx.send(message);
                }
            })?;
        let (read_only, startup_warning) = ready_rx
            .recv()
            .map_err(|_| RepositoryError::WorkerStopped)?
            .map_err(RepositoryError::WorkerInitialization)?;
        Ok(Self {
            commands: commands_tx,
            errors: errors_rx,
            next_request_id: Arc::new(AtomicU64::new(1)),
            read_only,
            startup_warning,
        })
    }

    pub fn is_read_only(&self) -> bool {
        self.read_only
    }

    pub fn startup_warning(&self) -> Option<&str> {
        self.startup_warning.as_deref()
    }

    pub fn load(&self) -> Result<AppDataSnapshot, RepositoryError> {
        let (sender, receiver) = bounded(1);
        self.send(DatabaseCommand::Load(sender))?;
        receiver
            .recv()
            .map_err(|_| RepositoryError::WorkerStopped)?
            .map_err(RepositoryError::WorkerInitialization)
    }

    pub fn save_task(&self, task: Task) -> Result<(), RepositoryError> {
        self.execute(DatabaseOperation::SaveTask(task))
    }

    pub fn save_tasks(&self, tasks: Vec<Task>) -> Result<(), RepositoryError> {
        self.execute(DatabaseOperation::SaveTasks(tasks))
    }

    pub fn replace_all_tasks(&self, tasks: Vec<Task>) -> Result<(), RepositoryError> {
        self.execute(DatabaseOperation::ReplaceAllTasks(tasks))
    }

    pub fn move_task_to_trash(
        &self,
        id: TaskId,
        now: OffsetDateTime,
    ) -> Result<(), RepositoryError> {
        self.execute(DatabaseOperation::MoveTaskToTrash(id, now))
    }

    pub fn restore_task(&self, id: TaskId, now: OffsetDateTime) -> Result<(), RepositoryError> {
        self.execute(DatabaseOperation::RestoreTask(id, now))
    }

    pub fn save_project(&self, project: Project) -> Result<(), RepositoryError> {
        self.execute(DatabaseOperation::SaveProject(project))
    }

    pub fn save_projects(&self, projects: Vec<Project>) -> Result<(), RepositoryError> {
        self.execute(DatabaseOperation::SaveProjects(projects))
    }

    pub fn apply_history_state(
        &self,
        tasks: Option<Vec<Task>>,
        projects_to_save: Vec<Project>,
        projects_to_delete: Vec<ProjectId>,
        tags_to_save: Vec<Tag>,
        tags_to_delete: Vec<TagId>,
    ) -> Result<(), RepositoryError> {
        self.execute(DatabaseOperation::ApplyHistoryState {
            tasks,
            projects_to_save,
            projects_to_delete,
            tags_to_save,
            tags_to_delete,
        })
    }

    pub fn delete_project(&self, id: ProjectId) -> Result<(), RepositoryError> {
        self.execute(DatabaseOperation::DeleteProject(id))
    }

    pub fn save_tag(&self, tag: Tag) -> Result<(), RepositoryError> {
        self.execute(DatabaseOperation::SaveTag(tag))
    }

    pub fn delete_tag(&self, id: TagId) -> Result<(), RepositoryError> {
        self.execute(DatabaseOperation::DeleteTag(id))
    }

    pub fn save_view(&self, view: SavedView) -> Result<(), RepositoryError> {
        self.execute(DatabaseOperation::SaveView(view))
    }

    pub fn delete_view(&self, id: SavedViewId) -> Result<(), RepositoryError> {
        self.execute(DatabaseOperation::DeleteView(id))
    }

    pub fn purge_expired_trash(&self, now: OffsetDateTime) -> Result<(), RepositoryError> {
        self.execute(DatabaseOperation::PurgeExpiredTrash(now))
    }

    pub fn empty_trash(&self) -> Result<(), RepositoryError> {
        self.execute(DatabaseOperation::EmptyTrash)
    }

    pub fn create_backup(&self, destination: PathBuf) -> Result<(), RepositoryError> {
        self.execute(DatabaseOperation::CreateBackup(destination))
    }

    pub fn restore_backup(
        &self,
        source: PathBuf,
        safety_backup: PathBuf,
    ) -> Result<AppDataSnapshot, RepositoryError> {
        let (sender, receiver) = bounded(1);
        self.send(DatabaseCommand::RestoreBackup(
            source,
            safety_backup,
            sender,
        ))?;
        receiver
            .recv()
            .map_err(|_| RepositoryError::WorkerStopped)?
            .map_err(RepositoryError::WorkerInitialization)
    }

    pub fn export_csv(
        &self,
        destination: PathBuf,
        filter: TaskFilter,
        with_bom: bool,
    ) -> Result<(), RepositoryError> {
        self.execute(DatabaseOperation::ExportCsv(destination, filter, with_bom))
    }

    pub fn export_task_csv(
        &self,
        destination: PathBuf,
        tasks: Vec<Task>,
        with_bom: bool,
    ) -> Result<(), RepositoryError> {
        self.execute(DatabaseOperation::ExportTaskCsv(
            destination,
            tasks,
            with_bom,
        ))
    }

    pub fn export_json(&self, destination: PathBuf) -> Result<(), RepositoryError> {
        self.execute(DatabaseOperation::ExportJson(destination))
    }

    pub fn take_error(&self) -> Option<String> {
        self.errors.try_recv().ok()
    }

    pub fn flush(&self) -> Result<(), RepositoryError> {
        let (sender, receiver) = bounded(1);
        self.send(DatabaseCommand::Flush(sender))?;
        receiver.recv().map_err(|_| RepositoryError::WorkerStopped)
    }

    fn send(&self, command: DatabaseCommand) -> Result<(), RepositoryError> {
        self.commands
            .send(command)
            .map_err(|_| RepositoryError::WorkerStopped)
    }

    fn execute(&self, operation: DatabaseOperation) -> Result<(), RepositoryError> {
        if self.read_only {
            return Err(RepositoryError::ReadOnly);
        }
        let request_id = self.next_request_id.fetch_add(1, Ordering::Relaxed);
        let (sender, receiver) = bounded(1);
        self.send(DatabaseCommand::Execute {
            request_id,
            operation: Box::new(operation),
            response: sender,
        })?;
        let response = receiver
            .recv()
            .map_err(|_| RepositoryError::WorkerStopped)?;
        debug_assert_eq!(response.request_id, request_id);
        response.result.map_err(RepositoryError::WorkerOperation)
    }
}

fn run_worker(
    mut repository: SqliteRepository,
    commands: Receiver<DatabaseCommand>,
    errors: Sender<String>,
) {
    while let Ok(command) = commands.recv() {
        let result: Result<(), RepositoryError> = match command {
            DatabaseCommand::Load(sender) => {
                let result = load_snapshot(&repository).map_err(|error| error.to_string());
                let _ = sender.send(result);
                Ok(())
            }
            DatabaseCommand::Execute {
                request_id,
                operation,
                response,
            } => {
                let result = execute_operation(&mut repository, *operation)
                    .map_err(|error| error.to_string());
                if let Err(message) = &result {
                    let _ = errors.send(message.clone());
                }
                let _ = response.send(DatabaseResult { request_id, result });
                Ok(())
            }
            DatabaseCommand::RestoreBackup(source, safety_backup, sender) => {
                let result = repository
                    .restore_from_backup(&source, &safety_backup)
                    .and_then(|_| load_snapshot(&repository))
                    .map_err(|error| error.to_string());
                let failed = result.as_ref().err().cloned();
                let _ = sender.send(result);
                if let Some(message) = failed {
                    let _ = errors.send(message);
                }
                Ok(())
            }
            DatabaseCommand::Flush(sender) => {
                let _ = sender.send(());
                Ok(())
            }
        };
        if let Err(error) = result {
            let _ = errors.send(error.to_string());
        }
    }
}

fn execute_operation(
    repository: &mut SqliteRepository,
    operation: DatabaseOperation,
) -> Result<(), RepositoryError> {
    match operation {
        DatabaseOperation::SaveTask(task) => repository.save_task(&task),
        DatabaseOperation::SaveTasks(tasks) => repository.save_tasks(&tasks),
        DatabaseOperation::ReplaceAllTasks(tasks) => repository.replace_all_tasks(&tasks),
        DatabaseOperation::MoveTaskToTrash(id, now) => {
            repository.move_task_to_trash(id, now).map(|_| ())
        }
        DatabaseOperation::RestoreTask(id, now) => repository.restore_task(id, now).map(|_| ()),
        DatabaseOperation::SaveProject(project) => repository.save_project(&project),
        DatabaseOperation::SaveProjects(projects) => repository.save_projects(&projects),
        DatabaseOperation::ApplyHistoryState {
            tasks,
            projects_to_save,
            projects_to_delete,
            tags_to_save,
            tags_to_delete,
        } => repository.apply_history_state(
            tasks.as_deref(),
            &projects_to_save,
            &projects_to_delete,
            &tags_to_save,
            &tags_to_delete,
        ),
        DatabaseOperation::DeleteProject(id) => repository.delete_project(id).map(|_| ()),
        DatabaseOperation::SaveTag(tag) => repository.save_tag(&tag),
        DatabaseOperation::DeleteTag(id) => repository.delete_tag(id).map(|_| ()),
        DatabaseOperation::SaveView(view) => repository.save_view(&view),
        DatabaseOperation::DeleteView(id) => repository.delete_view(id).map(|_| ()),
        DatabaseOperation::PurgeExpiredTrash(now) => {
            repository.purge_expired_trash(now, 30).map(|_| ())
        }
        DatabaseOperation::EmptyTrash => repository.empty_trash().map(|_| ()),
        DatabaseOperation::CreateBackup(destination) => repository.create_backup(&destination),
        DatabaseOperation::ExportCsv(destination, filter, with_bom) => {
            repository.export_csv(&destination, &filter, with_bom)
        }
        DatabaseOperation::ExportTaskCsv(destination, tasks, with_bom) => {
            SqliteRepository::export_tasks_csv(&destination, &tasks, with_bom)
        }
        DatabaseOperation::ExportJson(destination) => repository.export_json(&destination),
    }
}

fn load_snapshot(repository: &SqliteRepository) -> Result<AppDataSnapshot, RepositoryError> {
    Ok(AppDataSnapshot {
        tasks: repository.list_all_tasks()?,
        projects: repository.list_projects()?,
        tags: repository.list_tags()?,
        saved_views: repository.list_views()?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_returns_after_data_is_committed() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("worker.sqlite3");
        let worker = DatabaseWorker::start(&path).unwrap();
        let task = Task::new("committed", OffsetDateTime::UNIX_EPOCH).unwrap();
        worker.save_task(task.clone()).unwrap();
        let snapshot = worker.load().unwrap();
        assert_eq!(snapshot.tasks, vec![task]);
    }

    #[test]
    fn newer_schema_falls_back_to_read_only_mode() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("newer.sqlite3");
        let connection = rusqlite::Connection::open(&path).unwrap();
        connection.pragma_update(None, "user_version", 999).unwrap();
        drop(connection);

        let worker = DatabaseWorker::start(&path).unwrap();
        assert!(worker.is_read_only());
        assert!(worker.startup_warning().is_some());
        let task = Task::new("read only", OffsetDateTime::UNIX_EPOCH).unwrap();
        assert!(matches!(
            worker.save_task(task),
            Err(RepositoryError::ReadOnly)
        ));
    }
}
