use hodoq::{
    domain::{
        Due, DueScope, GroupBy, Priority, Project, SavedBaseView, SavedView, SavedViewId,
        SortDirection, SortField, SortSpec, Tag, Task, TaskFilter, TaskStatus, ViewKind,
    },
    infrastructure::SqliteRepository,
};
use time::{Date, OffsetDateTime};

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
