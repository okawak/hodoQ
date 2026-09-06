use hodoq::{
    domain::{
        Project, ProjectId, SavedView, SavedViewId, SortSpec, Tag, TagId, Task, TaskFilter,
        ViewKind,
    },
    infrastructure::{RepositoryError, SqliteRepository},
};
use rusqlite::Connection;
use std::{fs, path::Path};
use time::OffsetDateTime;

#[test]
fn verified_backup_can_restore_all_data() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("tasks.sqlite3");
    let backup = directory.path().join("backup.sqlite3");
    let safety = directory.path().join("before-restore.sqlite3");
    let mut repository = SqliteRepository::open(&database).unwrap();
    let now = OffsetDateTime::UNIX_EPOCH;
    let project = Project::new("project", now);
    let tag = Tag::new("tag", now);
    let view = SavedView {
        id: SavedViewId::new(),
        name: "view".to_owned(),
        view_kind: ViewKind::List,
        filter: TaskFilter::default(),
        sort: vec![SortSpec::default()],
        group_by: None,
        sort_order: 0,
        created_at: now,
        updated_at: now,
    };
    let mut task = Task::new("before", now).unwrap();
    task.project_id = Some(project.id);
    task.tag_ids.push(tag.id);
    repository.save_project(&project).unwrap();
    repository.save_tag(&tag).unwrap();
    repository.save_view(&view).unwrap();
    repository.save_task(&task).unwrap();
    repository.create_backup(&backup).unwrap();

    task.set_title("after").unwrap();
    repository.save_task(&task).unwrap();
    repository.delete_view(view.id).unwrap();
    repository.delete_tag(tag.id).unwrap();
    repository.delete_project(project.id).unwrap();
    repository.restore_from_backup(&backup, &safety).unwrap();

    task.set_title("before").unwrap();
    assert_eq!(repository.task(task.id).unwrap(), Some(task));
    assert_eq!(repository.list_projects().unwrap(), vec![project]);
    assert_eq!(repository.list_tags().unwrap(), vec![tag]);
    assert_eq!(repository.list_views().unwrap(), vec![view]);
    assert!(SqliteRepository::integrity_check(&safety).unwrap());
    let previous = SqliteRepository::open_read_only(&safety).unwrap();
    assert_eq!(previous.list_all_tasks().unwrap()[0].title, "after");
}

#[test]
fn newer_backup_is_rejected_without_changing_current_data() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("tasks.sqlite3");
    let source = directory.path().join("newer.sqlite3");
    let safety = directory.path().join("safety.sqlite3");
    let mut repository = SqliteRepository::open(&database).unwrap();
    let task = Task::new("current", OffsetDateTime::UNIX_EPOCH).unwrap();
    repository.save_task(&task).unwrap();
    let source_connection = Connection::open(&source).unwrap();
    source_connection
        .pragma_update(None, "user_version", 999)
        .unwrap();
    drop(source_connection);

    assert!(matches!(
        repository.restore_from_backup(&source, &safety),
        Err(RepositoryError::NewerSchema { .. })
    ));
    assert_eq!(repository.task(task.id).unwrap(), Some(task));
    assert!(!safety.exists());
}

#[test]
fn existing_schema_is_backed_up_before_migration() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("tasks.sqlite3");
    let connection = Connection::open(&database).unwrap();
    connection
        .execute("CREATE TABLE legacy (value TEXT NOT NULL)", [])
        .unwrap();
    drop(connection);

    SqliteRepository::open(&database).unwrap();

    let backups = fs::read_dir(directory.path().join("backups"))
        .unwrap()
        .filter_map(Result::ok)
        .collect::<Vec<_>>();
    assert_eq!(backups.len(), 1);
    assert!(SqliteRepository::integrity_check(&backups[0].path()).unwrap());
}

#[test]
fn invalid_backup_is_rejected_before_touching_current_data() {
    for invalid in [
        "empty",
        "unrelated",
        "missing table",
        "bad date",
        "bad json",
        "foreign key",
        "undeclared foreign key",
        "undeclared orphan",
        "migration",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("current.sqlite3");
        let source = directory.path().join("invalid.sqlite3");
        let safety = directory.path().join("safety.sqlite3");
        let mut current = SqliteRepository::open(&database).unwrap();
        let task = Task::new("preserve me", OffsetDateTime::UNIX_EPOCH).unwrap();
        current.save_task(&task).unwrap();
        match invalid {
            "empty" => {
                Connection::open(&source).unwrap();
            }
            "unrelated" => {
                Connection::open(&source)
                    .unwrap()
                    .execute_batch("CREATE TABLE unrelated (value TEXT); PRAGMA user_version=2;")
                    .unwrap();
            }
            "migration" => {
                Connection::open(&source)
                    .unwrap()
                    .execute_batch("CREATE TABLE tasks (id TEXT); PRAGMA user_version=1;")
                    .unwrap();
            }
            "undeclared foreign key" | "undeclared orphan" => {
                let connection = Connection::open(&source).unwrap();
                connection
                    .execute_batch(&schema_without_foreign_keys())
                    .unwrap();
                drop(connection);
                let mut backup = SqliteRepository::open(&source).unwrap();
                backup.save_task(&task).unwrap();
                let sql = if invalid == "undeclared foreign key" {
                    "UPDATE tasks SET project_id='00000000-0000-0000-0000-000000000001'"
                } else {
                    "INSERT INTO task_tags (task_id, tag_id) VALUES ('00000000-0000-0000-0000-000000000001', '00000000-0000-0000-0000-000000000002')"
                };
                drop(backup);
                Connection::open(&source)
                    .unwrap()
                    .execute_batch(sql)
                    .unwrap();
            }
            _ => {
                let mut backup = SqliteRepository::open(&source).unwrap();
                backup.save_task(&task).unwrap();
                let sql = match invalid {
                    "missing table" => "DROP TABLE saved_views",
                    "bad date" => "UPDATE tasks SET due_kind='date', due_date='not a date'",
                    "bad json" => {
                        "INSERT INTO saved_views (id,name,view_kind,filter_json,created_at,updated_at) VALUES ('00000000-0000-0000-0000-000000000001','broken','list','not json',0,0)"
                    }
                    "foreign key" => {
                        "PRAGMA foreign_keys=OFF; UPDATE tasks SET project_id='00000000-0000-0000-0000-000000000001'"
                    }
                    _ => unreachable!(),
                };
                drop(backup);
                Connection::open(&source)
                    .unwrap()
                    .execute_batch(sql)
                    .unwrap();
            }
        }
        let original_source = fs::read(&source).unwrap();
        let result = current.restore_from_backup(&source, &safety);
        assert!(
            matches!(result, Err(RepositoryError::InvalidBackup(_))),
            "{invalid}: {result:?}"
        );
        assert_eq!(current.task(task.id).unwrap(), Some(task), "{invalid}");
        assert_eq!(
            fs::read(&source).unwrap(),
            original_source,
            "source changed: {invalid}"
        );
        assert!(
            !safety.exists(),
            "backup validation must precede all writes: {invalid}"
        );
    }
}

#[test]
fn old_backup_is_migrated_on_a_copy_and_preserves_source() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("v1.sqlite3");
    let mut backup = SqliteRepository::open(&source).unwrap();
    let task = Task::new("legacy", OffsetDateTime::UNIX_EPOCH).unwrap();
    backup.save_task(&task).unwrap();
    drop(backup);
    Connection::open(&source)
        .unwrap()
        .pragma_update(None, "user_version", 1)
        .unwrap();
    let original_source = fs::read(&source).unwrap();
    let database = directory.path().join("current.sqlite3");
    let mut current = SqliteRepository::open(&database).unwrap();
    let expected_version = schema_version(&database);
    current
        .restore_from_backup(&source, &directory.path().join("safety.sqlite3"))
        .unwrap();
    assert_eq!(current.task(task.id).unwrap(), Some(task));
    assert_eq!(schema_version(&database), expected_version);
    assert_eq!(fs::read(source).unwrap(), original_source);
}

#[test]
fn valid_empty_hodoq_backup_can_replace_current_data() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("empty-hodoq.sqlite3");
    let safety = directory.path().join("safety.sqlite3");
    drop(SqliteRepository::open(&source).unwrap());
    let mut current = SqliteRepository::open_in_memory().unwrap();
    let task = Task::new("preserve in safety backup", OffsetDateTime::UNIX_EPOCH).unwrap();
    current.save_task(&task).unwrap();
    current.restore_from_backup(&source, &safety).unwrap();
    assert!(current.list_all_tasks().unwrap().is_empty());
    assert_eq!(
        SqliteRepository::open_read_only(&safety)
            .unwrap()
            .task(task.id)
            .unwrap(),
        Some(task)
    );
}

#[test]
fn restore_rebuilds_constraints_missing_from_source_schema() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("without-foreign-keys.sqlite3");
    let safety = directory.path().join("safety.sqlite3");
    let connection = Connection::open(&source).unwrap();
    connection
        .execute_batch(&schema_without_foreign_keys())
        .unwrap();
    drop(connection);
    let source_before = fs::read(&source).unwrap();
    let mut repository = SqliteRepository::open_in_memory().unwrap();
    repository.restore_from_backup(&source, &safety).unwrap();
    let mut task = Task::new("invalid project", OffsetDateTime::UNIX_EPOCH).unwrap();
    task.project_id = Some(ProjectId::new());
    assert!(repository.save_task(&task).is_err());
    task.project_id = None;
    task.tag_ids.push(TagId::new());
    assert!(repository.save_task(&task).is_err());
    assert_eq!(fs::read(source).unwrap(), source_before);
}

fn schema_without_foreign_keys() -> String {
    include_str!("../../migrations/0001_initial.sql")
        .replace("REFERENCES projects(id) ON DELETE SET NULL", "")
        .replace("REFERENCES tasks(id) ON DELETE CASCADE", "")
        .replace("REFERENCES tags(id) ON DELETE CASCADE", "")
}

fn schema_version(path: &Path) -> i64 {
    Connection::open(path)
        .unwrap()
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap()
}
