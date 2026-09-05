use gpui::{TestAppContext, WindowHandle};
use time::OffsetDateTime;

use super::*;
use crate::{
    application::TaskApplication,
    domain::{Priority, SortDirection, SortField, SortSpec, TaskId, TaskStatus, ViewKind},
    infrastructure::{AppPaths, AppSettings, InstanceLock},
};

fn workspace(cx: &mut TestAppContext) -> (tempfile::TempDir, WindowHandle<Workspace>, [TaskId; 4]) {
    let directory = tempfile::tempdir().unwrap();
    let paths = AppPaths::resolve(Some(directory.path())).unwrap();
    let lock = InstanceLock::acquire(&paths.lock).unwrap();
    let worker = TaskApplication::start(&paths.database).unwrap();
    let now = OffsetDateTime::now_utc();
    let tasks = [
        ("A", Priority::Low, TaskStatus::Todo),
        ("B", Priority::High, TaskStatus::Doing),
        ("C", Priority::Low, TaskStatus::Todo),
        ("D", Priority::Low, TaskStatus::Doing),
    ]
    .into_iter()
    .enumerate()
    .map(|(index, (title, priority, status))| {
        let mut task = Task::new(title, now).unwrap();
        task.priority = priority;
        task.status = status;
        task.sort_order = index as i64;
        task
    })
    .collect::<Vec<_>>();
    let ids = tasks
        .iter()
        .map(|task| task.id)
        .collect::<Vec<_>>()
        .try_into()
        .unwrap();
    worker.save_tasks(tasks).unwrap();
    let snapshot = worker.load().unwrap();
    cx.update(gpui_component::init);
    let window = cx.add_window(move |window, cx| {
        Workspace::new(
            worker,
            snapshot,
            paths,
            AppSettings {
                active_view: "all".to_owned(),
                ..AppSettings::default()
            },
            lock,
            false,
            window,
            cx,
        )
    });
    (directory, window, ids)
}

fn task_ids(tasks: Vec<Task>) -> Vec<TaskId> {
    tasks.into_iter().map(|task| task.id).collect()
}

#[gpui::test]
fn list_keyboard_navigation_follows_displayed_priority_order(cx: &mut TestAppContext) {
    let (_directory, window, [a, b, c, d]) = workspace(cx);
    window
        .update(cx, |workspace, window, cx| {
            assert_eq!(task_ids(workspace.visible_tasks(cx)), [b, a, c, d]);
            workspace.move_selection(1, window, cx);
            assert_eq!(workspace.selected_task, Some(b));
            for id in [a, c, d, d] {
                workspace.move_selection(1, window, cx);
                assert_eq!(workspace.selected_task, Some(id));
            }
            for id in [c, a, b, b] {
                workspace.move_selection(-1, window, cx);
                assert_eq!(workspace.selected_task, Some(id));
            }
            window.remove_window();
        })
        .unwrap();
}

#[gpui::test]
fn list_shift_selection_and_bulk_updates_follow_displayed_range(cx: &mut TestAppContext) {
    let (_directory, window, [a, b, c, d]) = workspace(cx);
    window
        .update(cx, |workspace, window, cx| {
            workspace.handle_task_click(b, false, false, window, cx);
            workspace.handle_task_click(c, true, false, window, cx);
            let expected = [b, a, c].into_iter().collect();
            assert_eq!(workspace.selected_tasks, expected);

            // Reverse ranges must include the same visually intermediate task.
            workspace.handle_task_click(c, false, false, window, cx);
            workspace.handle_task_click(b, true, false, window, cx);
            assert_eq!(workspace.selected_tasks, expected);
            workspace.bulk_status(TaskStatus::Done, cx);
            let snapshot = workspace.worker.load().unwrap();
            for task in snapshot.tasks {
                let expected_status = if task.id == d {
                    TaskStatus::Doing
                } else {
                    TaskStatus::Done
                };
                assert_eq!(
                    task.status, expected_status,
                    "incorrect bulk target: {}",
                    task.title
                );
            }
            window.remove_window();
        })
        .unwrap();
}

#[gpui::test]
fn list_grouping_and_filters_share_the_selection_order(cx: &mut TestAppContext) {
    let (_directory, window, [a, b, c, d]) = workspace(cx);
    window
        .update(cx, |workspace, window, cx| {
            workspace.sort = vec![SortSpec {
                field: SortField::Manual,
                direction: SortDirection::Descending,
            }];
            workspace.group_by = Some(GroupBy::Status);
            // Preserve the configured order within groups, with high priority first.
            assert_eq!(task_ids(workspace.visible_tasks(cx)), [c, a, b, d]);
            workspace.select_task(a, window, cx);
            workspace.move_selection(1, window, cx);
            assert_eq!(workspace.selected_task, Some(b));
            workspace.handle_task_click(c, false, false, window, cx);
            workspace.handle_task_click(b, true, false, window, cx);
            assert_eq!(workspace.selected_tasks, [c, a, b].into_iter().collect());

            workspace.filter_statuses.insert(TaskStatus::Doing);
            assert_eq!(task_ids(workspace.visible_tasks(cx)), [b, d]);
            workspace.select_task(b, window, cx);
            workspace.move_selection(1, window, cx);
            assert_eq!(workspace.selected_task, Some(d));
            window.remove_window();
        })
        .unwrap();
}

#[gpui::test]
fn list_ordering_does_not_change_board_or_calendar_sorting(cx: &mut TestAppContext) {
    let (_directory, window, [a, b, c, d]) = workspace(cx);
    window
        .update(cx, |workspace, window, cx| {
            workspace.sort = vec![SortSpec {
                field: SortField::Title,
                direction: SortDirection::Descending,
            }];
            assert_eq!(task_ids(workspace.visible_tasks(cx)), [b, d, c, a]);
            workspace.group_by = Some(GroupBy::Status);
            for view_kind in [ViewKind::Board, ViewKind::Calendar] {
                workspace.view_kind = view_kind;
                assert_eq!(task_ids(workspace.visible_tasks(cx)), [d, c, b, a]);
            }
            window.remove_window();
        })
        .unwrap();
}

#[gpui::test]
fn list_row_reordering_moves_visible_neighbors_without_changing_other_priorities(
    cx: &mut TestAppContext,
) {
    let (_directory, window, [a, b, c, d]) = workspace(cx);
    window
        .update(cx, |workspace, window, cx| {
            workspace.set_task_priority(d, Priority::High, cx);
            let original = workspace.tasks.clone();
            assert_eq!(task_ids(workspace.visible_tasks(cx)), [b, d, a, c]);
            workspace.move_task_order(b, 1, cx);
            assert_eq!(task_ids(workspace.visible_tasks(cx)), [d, b, a, c]);
            for id in [a, c] {
                assert_eq!(
                    workspace.tasks.iter().find(|t| t.id == id),
                    original.iter().find(|t| t.id == id)
                );
            }
            let reordered = workspace.tasks.clone();
            workspace.move_task_order(b, 1, cx); // High-priority boundary.
            workspace.move_task_order(a, -1, cx);
            workspace.swap_task_order(b, a, cx); // Drag cannot cross that boundary either.
            assert_eq!(workspace.tasks, reordered);
            workspace.undo(cx);
            assert_eq!(task_ids(workspace.visible_tasks(cx)), [b, d, a, c]);
            workspace.redo(cx);
            assert_eq!(task_ids(workspace.visible_tasks(cx)), [d, b, a, c]);
            let stored = workspace.worker.load().unwrap();
            for task in &workspace.tasks {
                assert_eq!(
                    stored
                        .tasks
                        .iter()
                        .find(|t| t.id == task.id)
                        .unwrap()
                        .sort_order,
                    task.sort_order
                );
            }
            window.remove_window();
        })
        .unwrap();
}

#[gpui::test]
fn list_row_reordering_respects_filters_and_groups(cx: &mut TestAppContext) {
    let (_directory, window, [a, b, c, d]) = workspace(cx);
    window
        .update(cx, |workspace, window, cx| {
            let original = workspace.tasks.clone();
            workspace.filter_statuses.insert(TaskStatus::Todo);
            assert_eq!(task_ids(workspace.visible_tasks(cx)), [a, c]);
            workspace.move_task_order(a, 1, cx);
            assert_eq!(task_ids(workspace.visible_tasks(cx)), [c, a]);
            for id in [b, d] {
                assert_eq!(
                    workspace.tasks.iter().find(|t| t.id == id),
                    original.iter().find(|t| t.id == id)
                );
            }
            let filtered = workspace.tasks.clone();
            workspace.move_task_order(b, 1, cx); // Hidden source cannot reorder anything.
            assert_eq!(workspace.tasks, filtered);
            workspace.filter_statuses.clear();
            workspace.group_by = Some(GroupBy::Status);
            assert_eq!(task_ids(workspace.visible_tasks(cx)), [c, a, b, d]);
            workspace.move_task_order(a, 1, cx); // Group boundary.
            assert_eq!(workspace.tasks, filtered);
            workspace.move_task_order(c, 1, cx);
            assert_eq!(task_ids(workspace.visible_tasks(cx)), [a, c, b, d]);
            window.remove_window();
        })
        .unwrap();
}

#[gpui::test]
fn list_row_reordering_supports_descending_manual_sort_but_not_automatic_sort(
    cx: &mut TestAppContext,
) {
    let (_directory, window, [a, b, c, d]) = workspace(cx);
    window
        .update(cx, |workspace, window, cx| {
            workspace.sort = vec![SortSpec {
                field: SortField::Manual,
                direction: SortDirection::Descending,
            }];
            assert_eq!(task_ids(workspace.visible_tasks(cx)), [b, d, c, a]);
            workspace.move_task_order(d, 1, cx);
            assert_eq!(task_ids(workspace.visible_tasks(cx)), [b, c, d, a]);
            workspace.sort = vec![SortSpec {
                field: SortField::Title,
                direction: SortDirection::Ascending,
            }];
            let before = workspace.tasks.clone();
            workspace.move_task_order(a, 1, cx);
            workspace.swap_task_order(a, c, cx);
            assert_eq!(workspace.tasks, before);
            window.remove_window();
        })
        .unwrap();
}
