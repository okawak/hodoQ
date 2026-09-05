use super::due::{
    calendar_leading_days, due_input_with_time, due_time_options, picker_due_input_value,
};
use super::task_editor::apply_pending_edits;
use super::*;
use crate::domain::{Due, TaskFilter, task_query::compare_tasks};
use gpui_component::calendar::Date as PickerDate;

#[gpui::test]
fn workspace_gui_tree_constructs(cx: &mut gpui::TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let paths = AppPaths::resolve(Some(directory.path())).unwrap();
    let instance_lock = InstanceLock::acquire(&paths.lock).unwrap();
    let application = TaskApplication::start(&paths.database).unwrap();
    let task = Task::new("中央編集テスト", OffsetDateTime::now_utc()).unwrap();
    let task_id = task.id;
    application.save_task(task).unwrap();
    let snapshot = application.load().unwrap();
    cx.update(gpui_component::init);
    let window = cx.add_window(move |window, cx| {
        Workspace::new(
            application,
            snapshot,
            paths,
            AppSettings::default(),
            instance_lock,
            true,
            window,
            cx,
        )
    });

    window
        .update(cx, |workspace, window, cx| {
            let _tree = workspace.render(window, cx).into_any_element();
            assert_eq!(workspace.active_view, SmartView::All);
            assert_eq!(workspace.view_kind, ViewKind::List);
            assert!(!workspace.show_saved_views);
            workspace.select_task(task_id, window, cx);
            workspace.update_due_from_input("2026-09-05 14:30", window, cx);
            assert!(matches!(
                workspace.selected_task().unwrap().due,
                Due::DateTime(_)
            ));
            workspace.clear_due(window, cx);
            assert_eq!(workspace.selected_task().unwrap().due, Due::None);
            workspace.view_kind = ViewKind::Calendar;
            let _selected_calendar_tree = workspace.render(window, cx).into_any_element();
            assert!(workspace.save_and_close_selected_task(cx));
            assert!(workspace.selected_task.is_none());

            workspace.open_new_task_form(window, cx);
            assert!(workspace.new_task_draft.is_some());
            workspace.title_input.update(cx, |state, cx| {
                state.set_value("新規フォームテスト", window, cx);
            });
            workspace.memo_input.update(cx, |state, cx| {
                state.set_value("全項目から作成", window, cx);
            });
            workspace.due_input.update(cx, |state, cx| {
                state.set_value("2026-08-30", window, cx);
            });
            workspace.progress_input.update(cx, |state, cx| {
                state.set_value("25", window, cx);
            });
            workspace.set_new_task_priority(Priority::High, cx);
            let _new_task_calendar_tree = workspace.render(window, cx).into_any_element();
            assert!(workspace.create_task(cx));
            assert!(workspace.new_task_draft.is_none());
            let created = workspace
                .tasks
                .iter()
                .find(|task| task.title == "新規フォームテスト")
                .unwrap();
            assert_eq!(created.memo, "全項目から作成");
            assert_eq!(created.priority, Priority::High);
            assert_eq!(created.progress, 25);
            assert!(matches!(created.due, Due::Date(_)));

            workspace.select_task(task_id, window, cx);
            workspace.title_input.update(cx, |state, cx| {
                state.set_value("", window, cx);
            });
            assert!(!workspace.save_and_close_selected_task(cx));
            assert_eq!(workspace.selected_task, Some(task_id));
            window.remove_window();
        })
        .unwrap();
}

#[gpui::test]
fn task_detail_content_stays_inside_resizable_slot(cx: &mut gpui::TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let paths = AppPaths::resolve(Some(directory.path())).unwrap();
    let instance_lock = InstanceLock::acquire(&paths.lock).unwrap();
    let application = TaskApplication::start(&paths.database).unwrap();
    let mut task = Task::new("右端レイアウトテスト", OffsetDateTime::now_utc()).unwrap();
    task.memo = "長いテキスト\n\n".repeat(24);
    let task_id = task.id;
    application.save_task(task).unwrap();
    let snapshot = application.load().unwrap();
    let settings = AppSettings {
        detail_width: 280.0,
        ..AppSettings::default()
    };
    cx.update(gpui_component::init);
    let window = cx.add_window(move |window, cx| {
        let mut workspace = Workspace::new(
            application,
            snapshot,
            paths,
            settings,
            instance_lock,
            true,
            window,
            cx,
        );
        workspace.select_task(task_id, window, cx);
        workspace
    });

    cx.run_until_parked();
    let mut visual = gpui::VisualTestContext::from_window(*window, cx);
    visual.run_until_parked();
    let slot = visual
        .debug_bounds("task-detail-slot")
        .expect("task detail slot should be rendered");
    let workspace_body = visual
        .debug_bounds("workspace-body")
        .expect("workspace body should be rendered");
    let slot_left = f32::from(slot.origin.x);
    let slot_right = f32::from(slot.origin.x + slot.size.width);
    let workspace_right = f32::from(workspace_body.origin.x + workspace_body.size.width);
    assert!(
        (slot_right - workspace_right).abs() <= 0.5,
        "detail slot must end at the window content edge: slot_right={slot_right}px, workspace_right={workspace_right}px"
    );
    for selector in [
        "task-detail-panel",
        "task-memo-input",
        "due-control",
        "task-progress-presets",
    ] {
        let bounds = visual
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector} should be rendered"));
        assert!(f32::from(bounds.origin.x) >= slot_left - 0.5);
        assert!(
            f32::from(bounds.origin.x + bounds.size.width) <= slot_right + 0.5,
            "{selector} overflowed its slot: right={}px, slot_right={}px",
            f32::from(bounds.origin.x + bounds.size.width),
            slot_right
        );
    }
    let memo = visual
        .debug_bounds("task-memo-input")
        .expect("memo input should be rendered");
    let due = visual
        .debug_bounds("due-control")
        .expect("due control should be rendered");
    assert!(
        f32::from(memo.origin.y + memo.size.height) <= f32::from(due.origin.y),
        "long memo text must not overlap the due control"
    );
    let due_left = f32::from(due.origin.x);
    let due_right = f32::from(due.origin.x + due.size.width);
    let bounds = visual.debug_bounds("due-input-control").unwrap();
    assert!(f32::from(bounds.origin.x) >= due_left - 0.5);
    assert!(f32::from(bounds.origin.x + bounds.size.width) <= due_right + 0.5);
    visual.update(|window, _| window.remove_window());
}

#[gpui::test]
fn legacy_classifications_survive_editing_in_the_simple_workspace(cx: &mut gpui::TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let paths = AppPaths::resolve(Some(directory.path())).unwrap();
    let lock = InstanceLock::acquire(&paths.lock).unwrap();
    let now = OffsetDateTime::now_utc();
    let project = Project::new("旧プロジェクト", now);
    let tag = Tag::new("旧タグ", now);
    let mut task = Task::new("分類済みタスク", now).unwrap();
    task.project_id = Some(project.id);
    task.tag_ids = vec![tag.id];
    let view = SavedView {
        id: SavedViewId::new(),
        name: "旧分類ビュー".to_owned(),
        view_kind: ViewKind::List,
        filter: TaskFilter {
            project_ids: vec![project.id],
            ..TaskFilter::default()
        },
        sort: vec![SortSpec::default()],
        group_by: Some(GroupBy::Project),
        sort_order: 0,
        created_at: now,
        updated_at: now,
    };
    let mut repository = crate::infrastructure::SqliteRepository::open(&paths.database).unwrap();
    repository.save_project(&project).unwrap();
    repository.save_tag(&tag).unwrap();
    repository.save_task(&task).unwrap();
    repository
        .save_task(&Task::new("分類なしタスク", now).unwrap())
        .unwrap();
    repository.save_view(&view).unwrap();
    drop(repository);
    let application = TaskApplication::start(&paths.database).unwrap();
    let snapshot = application.load().unwrap();
    let original_projects = snapshot.projects.clone();
    let original_tags = snapshot.tags.clone();
    let original_views = snapshot.saved_views.clone();
    assert_eq!(
        smart_view_from_setting(&format!("project:{}", project.id)),
        SmartView::All
    );
    assert_eq!(
        smart_view_from_setting(&format!("tag:{}", tag.id)),
        SmartView::All
    );
    let settings = AppSettings {
        active_view: format!("saved:{}", view.id),
        group_by: Some(GroupBy::Project),
        ..AppSettings::default()
    };
    cx.update(gpui_component::init);
    let window = cx.add_window(move |window, cx| {
        Workspace::new(
            application,
            snapshot,
            paths,
            settings,
            lock,
            false,
            window,
            cx,
        )
    });
    window
        .update(cx, |workspace, window, cx| {
            assert_eq!(workspace.active_view, SmartView::All);
            assert_eq!(workspace.group_by, None);
            assert_eq!(workspace.visible_tasks(cx).len(), 2);
            workspace.select_task(task.id, window, cx);
            workspace.title_input.update(cx, |state, cx| {
                state.set_value("変更後のタスク", window, cx)
            });
            assert!(workspace.save_and_close_selected_task(cx));
            let reloaded = workspace.worker.load().unwrap();
            let edited = reloaded
                .tasks
                .iter()
                .find(|item| item.id == task.id)
                .unwrap();
            assert_eq!(edited.title, "変更後のタスク");
            assert_eq!(edited.project_id, task.project_id);
            assert_eq!(edited.tag_ids, task.tag_ids);
            assert_eq!(reloaded.projects, original_projects);
            assert_eq!(reloaded.tags, original_tags);
            assert_eq!(reloaded.saved_views, original_views);
            window.remove_window();
        })
        .unwrap();
}

#[gpui::test]
fn editor_actions_remain_separated_in_a_short_narrow_pane(cx: &mut gpui::TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let paths = AppPaths::resolve(Some(directory.path())).unwrap();
    let lock = InstanceLock::acquire(&paths.lock).unwrap();
    let application = TaskApplication::start(&paths.database).unwrap();
    let task = Task::new("ボタンの間隔", OffsetDateTime::now_utc()).unwrap();
    let id = task.id;
    application.save_task(task).unwrap();
    let snapshot = application.load().unwrap();
    cx.update(gpui_component::init);
    let window = cx.add_window(move |window, cx| {
        let mut workspace = Workspace::new(
            application,
            snapshot,
            paths,
            AppSettings {
                detail_width: 280.0,
                ..AppSettings::default()
            },
            lock,
            true,
            window,
            cx,
        );
        workspace.select_task(id, window, cx);
        workspace
    });
    cx.run_until_parked();
    let mut visual = gpui::VisualTestContext::from_window(*window, cx);
    visual.simulate_resize(gpui::size(px(900.0), px(600.0)));
    visual.run_until_parked();
    let pane = visual.debug_bounds("task-detail-slot").unwrap();
    visual.simulate_event(gpui::ScrollWheelEvent {
        position: gpui::point(
            pane.origin.x + px(12.0),
            pane.origin.y + pane.size.height - px(12.0),
        ),
        delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.0), px(-2000.0))),
        ..Default::default()
    });
    visual.run_until_parked();
    let save = visual.debug_bounds("task-save-button").unwrap();
    let archive = visual.debug_bounds("task-archive-button").unwrap();
    let trash = visual.debug_bounds("task-trash-button").unwrap();
    for button in [save, archive, trash] {
        assert!(button.size.height >= px(24.0));
        assert!(button.origin.x >= pane.origin.x);
        assert!(button.right() <= pane.right());
        assert!(button.origin.y >= pane.origin.y && button.bottom() <= pane.bottom());
    }
    assert!(
        archive.origin.y - save.bottom() >= px(8.0),
        "save/archive gap: {:?}",
        archive.origin.y - save.bottom()
    );
    assert!(
        trash.origin.y - save.bottom() >= px(8.0),
        "save/trash gap: {:?}",
        trash.origin.y - save.bottom()
    );
    assert!(
        trash.origin.x - archive.right() >= px(8.0) || trash.origin.y - archive.bottom() >= px(8.0)
    );
    visual.update(|window, _| window.remove_window());
}

#[gpui::test]
fn list_rows_use_the_same_available_width(cx: &mut gpui::TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let paths = AppPaths::resolve(Some(directory.path())).unwrap();
    let instance_lock = InstanceLock::acquire(&paths.lock).unwrap();
    let application = TaskApplication::start(&paths.database).unwrap();
    let now = OffsetDateTime::now_utc();
    let mut undated = Task::new("納期なし", now).unwrap();
    undated.priority = Priority::High;
    let mut dated = Task::new("日付あり", now).unwrap();
    dated.due = Due::Date(Date::from_calendar_date(2026, time::Month::August, 31).unwrap());
    application.save_task(undated).unwrap();
    application.save_task(dated).unwrap();
    let snapshot = application.load().unwrap();
    cx.update(gpui_component::init);
    let window = cx.add_window(move |window, cx| {
        Workspace::new(
            application,
            snapshot,
            paths,
            AppSettings::default(),
            instance_lock,
            true,
            window,
            cx,
        )
    });

    cx.run_until_parked();
    let mut visual = gpui::VisualTestContext::from_window(*window, cx);
    visual.run_until_parked();
    let undated_bounds = visual
        .debug_bounds("task-row-undated")
        .expect("undated task row should be rendered");
    let dated_bounds = visual
        .debug_bounds("task-row-dated")
        .expect("dated task row should be rendered");
    assert!(
        (f32::from(undated_bounds.size.width) - f32::from(dated_bounds.size.width)).abs() <= 0.5,
        "list rows must have equal widths: undated={}px, dated={}px",
        f32::from(undated_bounds.size.width),
        f32::from(dated_bounds.size.width)
    );
    visual.update(|window, _| window.remove_window());
}

#[gpui::test]
fn calendar_month_grid_keeps_visible_height(cx: &mut gpui::TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let paths = AppPaths::resolve(Some(directory.path())).unwrap();
    let instance_lock = InstanceLock::acquire(&paths.lock).unwrap();
    let application = TaskApplication::start(&paths.database).unwrap();
    let snapshot = application.load().unwrap();
    cx.update(gpui_component::init);
    let window = cx.add_window(move |window, cx| {
        let mut workspace = Workspace::new(
            application,
            snapshot,
            paths,
            AppSettings::default(),
            instance_lock,
            true,
            window,
            cx,
        );
        workspace.view_kind = ViewKind::Calendar;
        workspace
    });

    cx.run_until_parked();
    let mut visual = gpui::VisualTestContext::from_window(*window, cx);
    visual.run_until_parked();
    let bounds = visual
        .debug_bounds("calendar-month-grid")
        .expect("calendar month grid should be rendered");
    assert!(
        f32::from(bounds.size.height) >= CALENDAR_GRID_MIN_HEIGHT,
        "calendar grid height was {}px",
        f32::from(bounds.size.height)
    );
    visual.update(|window, _| window.remove_window());
}

#[test]
fn due_input_supports_none_date_and_datetime() {
    assert_eq!(parse_due("未定").unwrap(), Due::None);
    assert!(matches!(parse_due("2026-08-28").unwrap(), Due::Date(_)));
    assert!(matches!(
        parse_due("2026-08-28 14:30").unwrap(),
        Due::DateTime(_)
    ));
}

#[test]
fn date_picker_selection_updates_due_input() {
    let selected = chrono::NaiveDate::from_ymd_opt(2026, 8, 30).unwrap();
    assert_eq!(
        picker_due_input_value(PickerDate::from(selected), ""),
        "2026-08-30"
    );
    assert_eq!(
        picker_due_input_value(PickerDate::from(selected), "2026-08-20 14:30"),
        "2026-08-30 14:30"
    );
    assert_eq!(
        picker_due_input_value(PickerDate::Single(None), "2026-08-20 14:30"),
        ""
    );
}

#[test]
fn time_selection_updates_the_unified_due_input() {
    assert_eq!(
        due_input_with_time("2026-08-30", Some("14:30")).unwrap(),
        "2026-08-30 14:30"
    );
    assert_eq!(
        due_input_with_time("2026-08-30 14:30", None).unwrap(),
        "2026-08-30"
    );
    assert!(due_input_with_time("", Some("14:30")).is_err());
    assert_eq!(due_time_options().len(), 96);
}

#[test]
fn calendar_month_starts_on_sunday() {
    let sunday = Date::from_calendar_date(2026, time::Month::November, 1).unwrap();
    let saturday = Date::from_calendar_date(2026, time::Month::August, 1).unwrap();
    assert_eq!(calendar_leading_days(sunday), 0);
    assert_eq!(calendar_leading_days(saturday), 6);
}

#[test]
fn pending_edits_are_validated_and_applied_before_close() {
    let now = OffsetDateTime::UNIX_EPOCH;
    let task = Task::new("before", now).unwrap();
    let id = task.id;
    let mut tasks = vec![task];
    let changed = apply_pending_edits(
        &mut tasks,
        Some(&(id, "after".to_owned())),
        Some(&(id, "memo".to_owned())),
        now + time::Duration::seconds(1),
    )
    .unwrap();
    assert!(changed);
    assert_eq!(tasks[0].title, "after");
    assert_eq!(tasks[0].memo, "memo");

    let snapshot = tasks.clone();
    assert!(
        apply_pending_edits(
            &mut tasks,
            Some(&(id, "   ".to_owned())),
            None,
            now + time::Duration::seconds(2),
        )
        .is_err()
    );
    assert_eq!(tasks, snapshot);
}

#[test]
fn ten_thousand_task_visible_search_finds_matching_task() {
    check_ten_thousand_task_visible_search();
}

#[test]
#[ignore = "run performance_ tests in release mode with --test-threads=1"]
#[allow(clippy::assertions_on_constants)]
fn performance_ten_thousand_task_visible_search() {
    // An explicit --ignored debug run should fail, not report misleading timings.
    assert!(
        !cfg!(debug_assertions),
        "performance tests require --release"
    );
    let elapsed = check_ten_thousand_task_visible_search();
    eprintln!("10,000 task visible search: {elapsed:?}");
    assert!(
        elapsed < StdDuration::from_millis(100),
        "10,000 task visible search took {elapsed:?}"
    );
}

fn check_ten_thousand_task_visible_search() -> StdDuration {
    let now = OffsetDateTime::UNIX_EPOCH;
    let tasks = (0..10_000)
        .map(|index| Task::new(format!("task {index}"), now).unwrap())
        .collect::<Vec<_>>();
    let started = std::time::Instant::now();
    let mut matches = tasks
        .iter()
        .filter(|task| {
            task.title.to_lowercase().contains("task 9999")
                || task.memo.to_lowercase().contains("task 9999")
        })
        .cloned()
        .collect::<Vec<_>>();
    matches
        .sort_by(|left, right| compare_tasks(left, right, &[SortSpec::default()], UtcOffset::UTC));
    let elapsed = started.elapsed();

    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].id, tasks[9999].id);
    elapsed
}
