use hodoq::{
    domain::Task,
    infrastructure::{DatabaseWorker, RepositoryError},
};
use time::OffsetDateTime;

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
