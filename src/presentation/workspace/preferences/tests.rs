use super::*;
use crate::{
    application::TaskApplication,
    domain::{SavedView, SavedViewId, SortSpec, TaskFilter},
    infrastructure::{AppPaths, AppSettings, InstanceLock},
    presentation::workspace::SmartView,
};
use time::OffsetDateTime;

#[gpui::test]
fn view_choice_is_saved_immediately_and_restored_on_restart(cx: &mut gpui::TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let paths = AppPaths::resolve(Some(directory.path())).unwrap();
    cx.update(gpui_component::init);

    for (expected, next) in [
        (ViewKind::List, ViewKind::Board),
        (ViewKind::Board, ViewKind::Calendar),
        (ViewKind::Calendar, ViewKind::List),
        (ViewKind::List, ViewKind::List),
    ] {
        let settings = AppSettings::load(&paths.settings);
        let worker = TaskApplication::start(&paths.database).unwrap();
        let snapshot = worker.load().unwrap();
        let lock = InstanceLock::acquire(&paths.lock).unwrap();
        let window_paths = paths.clone();
        let window = cx.add_window(move |window, cx| {
            Workspace::new(
                worker,
                snapshot,
                window_paths,
                settings,
                lock,
                false,
                window,
                cx,
            )
        });
        window
            .update(cx, |workspace, window, cx| {
                assert_eq!(workspace.view_kind, expected);
                workspace.set_view_kind(next, cx);
                // Read the actual file while the workspace is still open.
                assert_eq!(AppSettings::load(&paths.settings).view_kind, next);
                window.remove_window();
            })
            .unwrap();
        cx.run_until_parked();
    }
}

#[gpui::test]
fn restart_keeps_last_mode_without_overwriting_saved_view_preset(cx: &mut gpui::TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let paths = AppPaths::resolve(Some(directory.path())).unwrap();
    let worker = TaskApplication::start(&paths.database).unwrap();
    let now = OffsetDateTime::now_utc();
    let saved = SavedView {
        id: SavedViewId::new(),
        name: "作業中".to_owned(),
        view_kind: ViewKind::Board,
        filter: TaskFilter::default(),
        sort: vec![SortSpec::default()],
        group_by: None,
        sort_order: 0,
        created_at: now,
        updated_at: now,
    };
    worker.save_view(saved.clone()).unwrap();
    let snapshot = worker.load().unwrap();
    let lock = InstanceLock::acquire(&paths.lock).unwrap();
    let window_paths = paths.clone();
    cx.update(gpui_component::init);
    let window = cx.add_window(move |window, cx| {
        Workspace::new(
            worker,
            snapshot,
            window_paths,
            AppSettings::default(),
            lock,
            false,
            window,
            cx,
        )
    });
    window
        .update(cx, |workspace, window, cx| {
            workspace.activate_view(SmartView::Saved(saved.id), window, cx);
            assert_eq!(workspace.view_kind, ViewKind::Board);
            workspace.set_view_kind(ViewKind::List, cx);
            assert_eq!(workspace.saved_views[0].view_kind, ViewKind::Board);
            let settings = AppSettings::load(&paths.settings);
            assert_eq!(settings.active_view, format!("saved:{}", saved.id));
            assert_eq!(settings.view_kind, ViewKind::List);
            window.remove_window();
        })
        .unwrap();
    cx.run_until_parked();

    let settings = AppSettings::load(&paths.settings);
    let worker = TaskApplication::start(&paths.database).unwrap();
    let snapshot = worker.load().unwrap();
    let lock = InstanceLock::acquire(&paths.lock).unwrap();
    let window = cx.add_window(move |window, cx| {
        Workspace::new(worker, snapshot, paths, settings, lock, false, window, cx)
    });
    window
        .update(cx, |workspace, window, cx| {
            assert_eq!(workspace.active_view, SmartView::Saved(saved.id));
            assert_eq!(workspace.view_kind, ViewKind::List);
            workspace.activate_view(SmartView::All, window, cx);
            workspace.activate_view(SmartView::Saved(saved.id), window, cx);
            assert_eq!(workspace.view_kind, ViewKind::Board);
            window.remove_window();
        })
        .unwrap();
}

#[gpui::test]
fn settings_save_failure_recovers_without_clearing_unrelated_errors(cx: &mut gpui::TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let paths = AppPaths::resolve(Some(directory.path())).unwrap();
    let worker = TaskApplication::start(&paths.database).unwrap();
    let snapshot = worker.load().unwrap();
    let lock = InstanceLock::acquire(&paths.lock).unwrap();
    cx.update(gpui_component::init);
    let window = cx.add_window(move |window, cx| {
        Workspace::new(
            worker,
            snapshot,
            paths,
            AppSettings::default(),
            lock,
            false,
            window,
            cx,
        )
    });
    window
        .update(cx, |workspace, window, cx| {
            let original_path = workspace.paths.settings.clone();
            workspace.paths.settings = directory.path().join("missing").join("settings.json");
            workspace.set_view_kind(ViewKind::Board, cx);
            assert_eq!(workspace.view_kind, ViewKind::Board);
            assert!(
                workspace
                    .error_message
                    .as_deref()
                    .unwrap()
                    .contains("表示設定の保存に失敗")
            );
            workspace.paths.settings = original_path;
            workspace.set_view_kind(ViewKind::Calendar, cx);
            assert!(workspace.error_message.is_none());
            assert_eq!(
                AppSettings::load(&workspace.paths.settings).view_kind,
                ViewKind::Calendar
            );

            let unrelated_error = "納期の入力形式を確認してください".to_owned();
            workspace.error_message = Some(unrelated_error.clone());
            workspace.set_view_kind(ViewKind::List, cx);
            assert_eq!(workspace.error_message, Some(unrelated_error));
            assert_eq!(
                AppSettings::load(&workspace.paths.settings).view_kind,
                ViewKind::List
            );
            window.remove_window();
        })
        .unwrap();
}
