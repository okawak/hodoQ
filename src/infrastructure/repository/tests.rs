use super::*;
use crate::domain::Priority;
use crate::domain::{
    Due, GroupBy, Project, SavedView, SavedViewId, SortSpec, Tag, TaskFilter, TaskStatus, ViewKind,
    task_query::TaskQuery,
};
use crate::domain::{DueScope, SavedBaseView, SortDirection, SortField};
use std::fs;
use time::Date;

#[test]
fn task_round_trip_preserves_due_variants() {
    let mut repository = SqliteRepository::open_in_memory().unwrap();
    let now = OffsetDateTime::UNIX_EPOCH;
    let mut task = Task::new("日付タスク", now).unwrap();
    task.due = Due::Date(Date::from_calendar_date(2026, time::Month::August, 27).unwrap());
    repository.save_task(&task).unwrap();
    assert_eq!(repository.task(task.id).unwrap().unwrap(), task);

    task.due = Due::DateTime(now + time::Duration::hours(3));
    repository.save_task(&task).unwrap();
    assert_eq!(repository.task(task.id).unwrap().unwrap(), task);
}

#[test]
fn trash_is_purged_only_after_retention_period() {
    let mut repository = SqliteRepository::open_in_memory().unwrap();
    let now = OffsetDateTime::UNIX_EPOCH + time::Duration::days(100);
    let mut recent = Task::new("recent", now).unwrap();
    recent.deleted_at = Some(now - time::Duration::days(29));
    repository.save_task(&recent).unwrap();
    let mut expired = Task::new("expired", now).unwrap();
    expired.deleted_at = Some(now - time::Duration::days(30));
    repository.save_task(&expired).unwrap();

    assert_eq!(repository.purge_expired_trash(now, 30).unwrap(), 1);
    assert!(repository.task(recent.id).unwrap().is_some());
    assert!(repository.task(expired.id).unwrap().is_none());
}

#[test]
fn deleting_project_keeps_task() {
    let mut repository = SqliteRepository::open_in_memory().unwrap();
    let now = OffsetDateTime::UNIX_EPOCH;
    let project = Project::new("project", now);
    repository.save_project(&project).unwrap();
    let mut task = Task::new("task", now).unwrap();
    task.project_id = Some(project.id);
    repository.save_task(&task).unwrap();

    repository.delete_project(project.id).unwrap();
    let task = repository.task(task.id).unwrap().unwrap();
    assert_eq!(task.project_id, None);
}

#[test]
fn foreign_keys_reject_unknown_project_and_tag_without_partial_task() {
    let mut repository = SqliteRepository::open_in_memory().unwrap();
    let now = OffsetDateTime::UNIX_EPOCH;
    let unknown_project = Project::new("not saved", now);
    let unknown_tag = Tag::new("not saved", now);
    let mut task = Task::new("task", now).unwrap();
    task.project_id = Some(unknown_project.id);
    task.tag_ids.push(unknown_tag.id);

    assert!(repository.save_task(&task).is_err());
    assert!(repository.task(task.id).unwrap().is_none());
}

#[test]
fn batch_save_is_atomic() {
    let mut repository = SqliteRepository::open_in_memory().unwrap();
    let now = OffsetDateTime::UNIX_EPOCH;
    let valid = Task::new("valid", now).unwrap();
    let mut invalid = Task::new("invalid", now).unwrap();
    invalid.title.clear();

    assert!(repository.save_tasks(&[valid.clone(), invalid]).is_err());
    assert!(repository.task(valid.id).unwrap().is_none());
}

#[test]
fn project_batch_save_is_atomic() {
    let mut repository = SqliteRepository::open_in_memory().unwrap();
    let now = OffsetDateTime::UNIX_EPOCH;
    let valid = Project::new("valid", now);
    let invalid = Project::new("", now);

    assert!(repository.save_projects(&[valid.clone(), invalid]).is_err());
    assert!(repository.list_projects().unwrap().is_empty());
}

#[test]
fn history_state_is_atomic_across_related_entities() {
    let mut repository = SqliteRepository::open_in_memory().unwrap();
    let now = OffsetDateTime::UNIX_EPOCH;
    let project = Project::new("project", now);
    let mut invalid_task = Task::new("task", now).unwrap();
    invalid_task.title.clear();
    invalid_task.project_id = Some(project.id);

    assert!(
        repository
            .apply_history_state(
                Some(&[invalid_task]),
                std::slice::from_ref(&project),
                &[],
                &[],
                &[],
            )
            .is_err()
    );
    assert!(repository.list_projects().unwrap().is_empty());
    assert!(repository.list_all_tasks().unwrap().is_empty());
}

#[test]
fn history_state_restores_project_tag_and_task_relationships_together() {
    let mut repository = SqliteRepository::open_in_memory().unwrap();
    let now = OffsetDateTime::UNIX_EPOCH;
    let project = Project::new("project", now);
    let tag = Tag::new("tag", now);
    let mut task = Task::new("task", now).unwrap();
    task.project_id = Some(project.id);
    task.tag_ids.push(tag.id);

    repository
        .apply_history_state(
            Some(&[task.clone()]),
            std::slice::from_ref(&project),
            &[],
            std::slice::from_ref(&tag),
            &[],
        )
        .unwrap();

    assert_eq!(repository.list_projects().unwrap(), vec![project]);
    assert_eq!(repository.list_tags().unwrap(), vec![tag]);
    assert_eq!(repository.task(task.id).unwrap(), Some(task));
}

#[test]
fn list_all_tasks_includes_trash() {
    let mut repository = SqliteRepository::open_in_memory().unwrap();
    let now = OffsetDateTime::UNIX_EPOCH;
    let active = Task::new("active", now).unwrap();
    let mut deleted = Task::new("deleted", now).unwrap();
    deleted.move_to_trash(now);
    repository.save_tasks(&[active, deleted]).unwrap();

    let tasks = repository.list_all_tasks().unwrap();
    assert_eq!(tasks.len(), 2);
    assert_eq!(
        tasks
            .iter()
            .filter(|task| task.deleted_at.is_some())
            .count(),
        1
    );
}

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
fn saved_view_round_trip_preserves_conditions() {
    let mut repository = SqliteRepository::open_in_memory().unwrap();
    let now = OffsetDateTime::UNIX_EPOCH;
    let project = Project::new("project", now);
    let view = SavedView {
        id: SavedViewId::new(),
        name: "重要".to_owned(),
        view_kind: ViewKind::Board,
        filter: TaskFilter {
            base_view: Some(SavedBaseView::Project(project.id)),
            priorities: vec![Priority::High],
            ..TaskFilter::default()
        },
        sort: vec![SortSpec {
            field: SortField::Due,
            direction: SortDirection::Ascending,
        }],
        group_by: Some(GroupBy::Status),
        sort_order: 0,
        created_at: now,
        updated_at: now,
    };
    repository.save_view(&view).unwrap();
    assert_eq!(repository.list_views().unwrap(), vec![view]);
}

#[test]
fn due_scope_filter_selects_undated_tasks() {
    let mut repository = SqliteRepository::open_in_memory().unwrap();
    let now = OffsetDateTime::now_utc();
    let undated = Task::new("undated", now).unwrap();
    let mut dated = Task::new("dated", now).unwrap();
    dated.due = Due::Date(now.date());
    repository.save_tasks(&[undated.clone(), dated]).unwrap();

    let tasks = repository
        .list_tasks(
            &TaskFilter {
                due_scope: DueScope::Undated,
                ..TaskFilter::default()
            },
            &[],
        )
        .unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].id, undated.id);
}

#[test]
fn overdue_filter_excludes_completed_tasks() {
    let mut repository = SqliteRepository::open_in_memory().unwrap();
    let now = OffsetDateTime::now_utc();
    let yesterday = now.date() - time::Duration::days(1);
    let mut open = Task::new("open", now).unwrap();
    open.due = Due::Date(yesterday);
    let mut done = Task::new("done", now).unwrap();
    done.due = Due::Date(yesterday);
    done.set_status(TaskStatus::Done, now);
    repository.save_tasks(&[open.clone(), done]).unwrap();

    let tasks = repository
        .list_tasks(
            &TaskFilter {
                due_scope: DueScope::Overdue,
                ..TaskFilter::default()
            },
            &[],
        )
        .unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].id, open.id);
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
fn ten_thousand_tasks_can_be_saved_and_loaded() {
    check_ten_thousand_task_round_trip();
}

#[test]
#[ignore = "run performance_ tests in release mode with --test-threads=1"]
#[allow(clippy::assertions_on_constants)]
fn performance_ten_thousand_task_round_trip() {
    // An explicit --ignored debug run should fail, not report misleading timings.
    assert!(
        !cfg!(debug_assertions),
        "performance tests require --release"
    );
    let (round_trip, load, search) = check_ten_thousand_task_round_trip();
    eprintln!("10,000 tasks: round trip={round_trip:?}, load={load:?}, search={search:?}");
    assert!(
        round_trip < Duration::from_secs(5),
        "10,000 task round trip took {round_trip:?}"
    );
    assert!(
        load < Duration::from_secs(1),
        "10,000 task load took {load:?}"
    );
    assert!(
        search < Duration::from_millis(100),
        "10,000 task search took {search:?}"
    );
}

// Keep the same dataset and correctness checks in functional and performance runs.
fn check_ten_thousand_task_round_trip() -> (Duration, Duration, Duration) {
    let mut repository = SqliteRepository::open_in_memory().unwrap();
    let now = OffsetDateTime::UNIX_EPOCH;
    let tasks = (0..10_000)
        .map(|index| Task::new(format!("task {index}"), now).unwrap())
        .collect::<Vec<_>>();
    let started = std::time::Instant::now();
    repository.save_tasks(&tasks).unwrap();
    let load_started = std::time::Instant::now();
    let loaded = repository.list_all_tasks().unwrap();
    let load_elapsed = load_started.elapsed();
    let round_trip_elapsed = started.elapsed();

    assert_eq!(loaded.len(), 10_000);
    let search_started = std::time::Instant::now();
    let filter = TaskFilter {
        query: "task 9999".to_owned(),
        ..TaskFilter::default()
    };
    let matches = repository
        .list_tasks(&filter, &[SortSpec::default()])
        .unwrap();
    let search_elapsed = search_started.elapsed();
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].id, tasks[9999].id);
    (round_trip_elapsed, load_elapsed, search_elapsed)
}

#[test]
fn project_filter_can_include_selected_and_unassigned_tasks() {
    let now = OffsetDateTime::UNIX_EPOCH;
    let project = Project::new("project", now);
    let mut assigned = Task::new("assigned", now).unwrap();
    assigned.project_id = Some(project.id);
    let unassigned = Task::new("unassigned", now).unwrap();
    let filter = TaskFilter {
        project_ids: vec![project.id],
        unassigned_project: true,
        ..TaskFilter::default()
    };
    assert!(TaskQuery::new(&filter, now, time::UtcOffset::UTC).matches(&assigned));
    assert!(TaskQuery::new(&filter, now, time::UtcOffset::UTC).matches(&unassigned));
}

#[test]
fn saved_base_views_preserve_project_archive_and_trash_scope() {
    let now = OffsetDateTime::UNIX_EPOCH;
    let project = Project::new("project", now);
    let mut assigned = Task::new("assigned", now).unwrap();
    assigned.project_id = Some(project.id);
    let unassigned = Task::new("unassigned", now).unwrap();
    let project_filter = TaskFilter {
        base_view: Some(SavedBaseView::Project(project.id)),
        ..TaskFilter::default()
    };
    assert!(TaskQuery::new(&project_filter, now, time::UtcOffset::UTC).matches(&assigned));
    assert!(!TaskQuery::new(&project_filter, now, time::UtcOffset::UTC).matches(&unassigned));

    let mut archived = Task::new("archived", now).unwrap();
    archived.set_status(TaskStatus::Archived, now);
    let archive_filter = TaskFilter {
        base_view: Some(SavedBaseView::Archived),
        ..TaskFilter::default()
    };
    assert!(TaskQuery::new(&archive_filter, now, time::UtcOffset::UTC).matches(&archived));

    archived.move_to_trash(now);
    let trash_filter = TaskFilter {
        base_view: Some(SavedBaseView::Trash),
        ..TaskFilter::default()
    };
    assert!(TaskQuery::new(&trash_filter, now, time::UtcOffset::UTC).matches(&archived));
}

#[test]
fn csv_export_has_optional_bom_and_consistent_columns() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("tasks.csv");
    let mut task = Task::new("comma, title", OffsetDateTime::UNIX_EPOCH).unwrap();
    task.memo = "line 1\nline 2".to_owned();
    SqliteRepository::export_tasks_csv(&path, &[task], true).unwrap();
    let bytes = fs::read(&path).unwrap();
    assert!(bytes.starts_with(&[0xEF, 0xBB, 0xBF]));
    let mut reader = csv::Reader::from_reader(bytes.as_slice());
    assert_eq!(reader.headers().unwrap().len(), 11);
    assert_eq!(reader.records().next().unwrap().unwrap().len(), 11);

    SqliteRepository::export_tasks_csv(&path, &[], false).unwrap();
    assert!(!fs::read(path).unwrap().starts_with(&[0xEF, 0xBB, 0xBF]));
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
                backup.connection.execute_batch(sql).unwrap();
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
    backup
        .connection
        .pragma_update(None, "user_version", 1)
        .unwrap();
    drop(backup);
    let original_source = fs::read(&source).unwrap();
    let mut current = SqliteRepository::open_in_memory().unwrap();
    current
        .restore_from_backup(&source, &directory.path().join("safety.sqlite3"))
        .unwrap();
    assert_eq!(current.task(task.id).unwrap(), Some(task));
    assert_eq!(
        current
            .connection
            .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        migrations::CURRENT_SCHEMA_VERSION
    );
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
