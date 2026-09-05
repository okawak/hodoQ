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

#[gpui::test]
fn list_rows_pack_from_the_top_with_or_without_group_headings(cx: &mut TestAppContext) {
    let (_directory, window, _) = workspace(cx);
    let mut visual = gpui::VisualTestContext::from_window(*window, cx);
    for group in [None, Some(GroupBy::Status), Some(GroupBy::Priority), None] {
        window
            .update(&mut visual, |workspace, _, cx| {
                workspace.group_by = group;
                cx.notify();
            })
            .unwrap();
        for (width, height) in [(900., 1600.), (2040., 1640.), (1280., 800.)] {
            visual.simulate_resize(gpui::size(px(width), px(height)));
            visual.run_until_parked();
            let selectors = window
                .update(&mut visual, |workspace, _, cx| {
                    let mut selectors = Vec::new();
                    let mut last_group = None;
                    for task in workspace.visible_tasks(cx) {
                        if let Some(group) = group {
                            let label = workspace.task_group_label(&task, group);
                            if last_group.as_ref() != Some(&label) {
                                selectors.push(format!("task-group-{label}"));
                                last_group = Some(label);
                            }
                        }
                        selectors.push(format!("task-item-{}", task.id));
                    }
                    selectors
                })
                .unwrap();
            let bounds = selectors
                .into_iter()
                .map(|selector| {
                    visual
                        .debug_bounds(selector.leak())
                        .expect("all four rows and headings should fit")
                })
                .collect::<Vec<_>>();
            for pair in bounds.windows(2) {
                assert!(
                    (pair[1].top() - pair[0].bottom()).abs() <= px(0.5),
                    "no unused vertical space or overlap between items: {pair:?}"
                );
                assert_eq!(pair[0].size.width, pair[1].size.width);
            }
            assert!(
                bounds.last().unwrap().bottom() - bounds[0].top() <= px(640.),
                "four tasks and their headings must remain compact: {bounds:?}"
            );
        }
    }
    visual.update(|window, _| window.remove_window());
}

#[gpui::test]
fn large_list_stays_virtualized_and_keeps_scroll_position_on_refresh(cx: &mut TestAppContext) {
    let (_directory, window, _) = workspace(cx);
    let ids = window
        .update(cx, |workspace, _, cx| {
            workspace.tasks = (0..10_000)
                .map(|index| {
                    let mut task =
                        Task::new(format!("タスク {index}"), OffsetDateTime::now_utc()).unwrap();
                    task.sort_order = index;
                    task
                })
                .collect();
            cx.notify();
            workspace
                .tasks
                .iter()
                .map(|task| task.id)
                .collect::<Vec<_>>()
        })
        .unwrap();
    let mut visual = gpui::VisualTestContext::from_window(*window, cx);
    visual.simulate_resize(gpui::size(px(1280.), px(800.)));
    visual.run_until_parked();
    assert!(
        visual
            .debug_bounds(format!("task-item-{}", ids[0]).leak())
            .is_some()
    );
    assert!(
        visual
            .debug_bounds(format!("task-item-{}", ids[9999]).leak())
            .is_none()
    );
    window
        .update(&mut visual, |workspace, _, cx| {
            workspace.task_list_state.scroll_to(gpui::ListOffset {
                item_ix: 100,
                offset_in_item: px(0.),
            });
            cx.notify();
        })
        .unwrap();
    visual.run_until_parked();
    let selector = format!("task-item-{}", ids[100]).leak();
    let before = visual.debug_bounds(selector).unwrap();
    window.update(&mut visual, |_, _, cx| cx.notify()).unwrap();
    visual.run_until_parked();
    assert_eq!(visual.debug_bounds(selector).unwrap(), before);
    window
        .update(&mut visual, |workspace, _, cx| {
            // Removing/filtering to fewer rows must clamp a stale scroll offset.
            workspace.tasks.truncate(2);
            cx.notify();
        })
        .unwrap();
    visual.run_until_parked();
    assert!(
        visual
            .debug_bounds(format!("task-item-{}", ids[0]).leak())
            .is_some()
    );
    visual.update(|window, _| window.remove_window());
}

#[gpui::test]
fn list_metadata_and_actions_fit_with_editor_at_minimum_window_width(cx: &mut TestAppContext) {
    let (_directory, window, [a, b, _, _]) = workspace(cx);
    window
        .update(cx, |workspace, window, cx| {
            workspace.tasks.retain(|task| task.id == a || task.id == b);
            for task in &mut workspace.tasks {
                if task.id == a {
                    task.title = "とても長いタスク名".repeat(20);
                } else {
                    task.due = super::super::parse_due("2026-09-08 14:37").unwrap();
                    task.progress = 100;
                }
            }
            workspace.select_task(a, window, cx);
        })
        .unwrap();
    let mut visual = gpui::VisualTestContext::from_window(*window, cx);
    for width in [900., 1100., 1600., 900.] {
        visual.simulate_resize(gpui::size(px(width), px(1200.)));
        visual.run_until_parked();
        let undated = visual.debug_bounds("task-row-undated").unwrap();
        let dated = visual.debug_bounds("task-row-dated").unwrap();
        let editor = visual.debug_bounds("task-detail-slot").unwrap();
        assert!(
            dated.size.height <= px(220.),
            "row must remain compact: {dated:?}"
        );
        if width >= 1600. {
            assert!(
                dated.size.height <= px(160.),
                "wide rows must remain compact: {dated:?}"
            );
        }
        assert_eq!(undated.size.width, dated.size.width);
        assert!(
            dated.bottom() <= undated.top(),
            "task rows must not overlap"
        );
        assert!(
            (undated.top() - dated.bottom() - px(8.)).abs() <= px(0.5),
            "rows need only an 8px gap: {dated:?}, {undated:?}"
        );
        for (id, row) in [(a, undated), (b, dated)] {
            assert!(row.right() <= editor.left());
            for part in ["info", "title", "status", "priority", "due", "delete"] {
                let selector = format!("task-{part}-{id}").leak();
                let bounds = visual.debug_bounds(selector).unwrap();
                assert!(
                    bounds.left() >= row.left() && bounds.right() <= row.right(),
                    "{part} must fit row at {width}px: {bounds:?}, row={row:?}"
                );
                assert!(
                    bounds.top() >= row.top() && bounds.bottom() <= row.bottom(),
                    "{part} must fit row height at {width}px: {bounds:?}, row={row:?}"
                );
            }
        }
    }
    visual.update(|window, _| window.remove_window());
}

#[gpui::test]
fn saved_scope_and_live_filters_compose_without_losing_trash_or_archive(cx: &mut TestAppContext) {
    use super::super::SmartView;
    use crate::domain::{SavedView, SavedViewId, TaskFilter};
    let (_directory, window, [a, _, c, d]) = workspace(cx);
    window
        .update(cx, |workspace, window, cx| {
            workspace.set_task_status(a, TaskStatus::Archived, cx);
            let now = OffsetDateTime::now_utc();
            let view = SavedView {
                id: SavedViewId::new(),
                name: "archive included".into(),
                view_kind: ViewKind::List,
                filter: TaskFilter {
                    include_archived: true,
                    ..Default::default()
                },
                sort: vec![SortSpec::default()],
                group_by: None,
                sort_order: 0,
                created_at: now,
                updated_at: now,
            };
            workspace.active_view = SmartView::Saved(view.id);
            workspace.saved_views.push(view);
            workspace.filter_priorities.insert(Priority::Low);
            assert_eq!(task_ids(workspace.visible_tasks(cx)), [a, c, d]);
            workspace.filter_statuses.insert(TaskStatus::Todo);
            assert_eq!(task_ids(workspace.visible_tasks(cx)), [c]);
            workspace.move_to_trash(c, cx);
            workspace
                .saved_views
                .last_mut()
                .unwrap()
                .filter
                .only_deleted = true;
            assert_eq!(task_ids(workspace.visible_tasks(cx)), [c]);
            workspace.saved_views.last_mut().unwrap().filter.query = " C ".into();
            workspace
                .search_input
                .update(cx, |state, cx| state.set_value(" C ", window, cx));
            assert!(workspace.visible_tasks(cx).is_empty());
            workspace
                .search_input
                .update(cx, |state, cx| state.set_value("c", window, cx));
            assert_eq!(task_ids(workspace.visible_tasks(cx)), [c]);
            workspace.active_view = SmartView::Saved(SavedViewId::new());
            assert!(workspace.visible_tasks(cx).is_empty());
            window.remove_window();
        })
        .unwrap();
}

#[gpui::test]
fn undo_redo_acknowledge_storage_and_preserve_history_on_failure(cx: &mut TestAppContext) {
    let (_directory, window, [a, _, _, _]) = workspace(cx);
    window
        .update(cx, |workspace, window, cx| {
            workspace.set_task_status(a, TaskStatus::Done, cx);
            workspace.undo(cx);
            assert_eq!(
                workspace
                    .tasks
                    .iter()
                    .find(|task| task.id == a)
                    .unwrap()
                    .status,
                TaskStatus::Todo
            );
            assert_eq!(
                workspace
                    .worker
                    .load()
                    .unwrap()
                    .tasks
                    .iter()
                    .find(|task| task.id == a)
                    .unwrap()
                    .status,
                TaskStatus::Todo
            );
            workspace.redo(cx);
            let before = workspace.tasks.clone();
            let connection = rusqlite::Connection::open(&workspace.paths.database).unwrap();
            connection.pragma_update(None, "user_version", 999).unwrap();
            drop(connection);
            workspace.worker = TaskApplication::start(&workspace.paths.database).unwrap();
            assert!(workspace.worker.is_read_only());
            let undo_len = workspace.undo_stack.len();
            workspace.undo(cx);
            assert_eq!(workspace.tasks, before);
            assert_eq!(workspace.undo_stack.len(), undo_len);
            assert!(workspace.redo_stack.is_empty());
            assert!(workspace.error_message.is_some());
            assert_eq!(
                workspace
                    .worker
                    .load()
                    .unwrap()
                    .tasks
                    .iter()
                    .find(|task| task.id == a)
                    .unwrap()
                    .status,
                TaskStatus::Done
            );
            window.remove_window();
        })
        .unwrap();
}
