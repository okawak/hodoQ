use super::*;
use crate::{
    application::TaskApplication,
    infrastructure::{AppPaths, AppSettings, InstanceLock},
};
use gpui::{TestAppContext, WindowHandle};

fn workspace(cx: &mut TestAppContext) -> (tempfile::TempDir, WindowHandle<Workspace>, [TaskId; 2]) {
    let directory = tempfile::tempdir().unwrap();
    let paths = AppPaths::resolve(Some(directory.path())).unwrap();
    let lock = InstanceLock::acquire(&paths.lock).unwrap();
    let worker = TaskApplication::start(&paths.database).unwrap();
    let now = OffsetDateTime::UNIX_EPOCH;
    let tasks = [
        Task::new("first", now).unwrap(),
        Task::new("second", now).unwrap(),
    ];
    let ids = [tasks[0].id, tasks[1].id];
    worker.save_tasks(tasks.to_vec()).unwrap();
    let snapshot = worker.load().unwrap();
    cx.update(gpui_component::init);
    let window = cx.add_window(move |window, cx| {
        Workspace::new(
            worker,
            snapshot,
            paths,
            AppSettings::default(),
            lock,
            true,
            window,
            cx,
        )
    });
    (directory, window, ids)
}

#[gpui::test]
fn invalid_title_replaces_older_pending_text_and_blocks_close(cx: &mut TestAppContext) {
    let (_directory, window, [a, _]) = workspace(cx);
    window
        .update(cx, |workspace, window, cx| {
            workspace.select_task(a, window, cx);
            workspace.schedule_title_save("valid draft".into(), cx);
            let old_revision = workspace.title_revision;
            workspace.schedule_title_save("".into(), cx);
            assert_ne!(workspace.title_revision, old_revision);
            assert_eq!(workspace.pending_title, Some((a, String::new())));
            assert!(!workspace.should_close(cx));
            assert_eq!(
                workspace
                    .worker
                    .load()
                    .unwrap()
                    .tasks
                    .iter()
                    .find(|task| task.id == a)
                    .unwrap()
                    .title,
                "first"
            );
            workspace.schedule_title_save("corrected".into(), cx);
            assert!(workspace.persist_pending_edits().is_ok());
            assert_eq!(
                workspace
                    .worker
                    .load()
                    .unwrap()
                    .tasks
                    .iter()
                    .find(|task| task.id == a)
                    .unwrap()
                    .title,
                "corrected"
            );
            window.remove_window();
        })
        .unwrap();
}

#[gpui::test]
fn failed_edit_prevents_all_selection_paths_from_discarding_input(cx: &mut TestAppContext) {
    let (_directory, window, [a, b]) = workspace(cx);
    window
        .update(cx, |workspace, window, cx| {
            workspace.select_task(a, window, cx);
            workspace
                .title_input
                .update(cx, |input, cx| input.set_value("unsaved", window, cx));
            workspace.schedule_title_save("unsaved".into(), cx);
            workspace
                .memo_input
                .update(cx, |input, cx| input.set_value("unsaved memo", window, cx));
            workspace.schedule_memo_save("unsaved memo".into(), cx);
            let connection = rusqlite::Connection::open(&workspace.paths.database).unwrap();
            connection.pragma_update(None, "user_version", 999).unwrap();
            drop(connection);
            workspace.worker = TaskApplication::start(&workspace.paths.database).unwrap();
            assert!(workspace.worker.is_read_only());
            workspace.select_task(b, window, cx);
            assert_eq!(workspace.selected_task, Some(a));
            workspace.handle_task_click(b, true, false, window, cx);
            assert_eq!(workspace.selected_task, Some(a));
            workspace.handle_task_click(b, false, true, window, cx);
            assert_eq!(workspace.selected_task, Some(a));
            workspace.open_new_task_form(window, cx);
            assert!(workspace.new_task_draft.is_none());
            assert_eq!(workspace.selected_task, Some(a));
            let active_view = workspace.active_view;
            workspace.activate_view(SmartView::Today, window, cx);
            assert_eq!(workspace.active_view, active_view);
            assert_eq!(workspace.selected_task, Some(a));
            workspace.duplicate_task(b, window, cx);
            assert_eq!(workspace.selected_task, Some(a));
            assert_eq!(workspace.tasks.len(), 2);
            workspace.close_task_form(cx);
            assert_eq!(workspace.selected_task, Some(a));
            assert_eq!(workspace.pending_title, Some((a, "unsaved".into())));
            assert_eq!(workspace.title_input.read(cx).value().as_str(), "unsaved");
            assert_eq!(workspace.pending_memo, Some((a, "unsaved memo".into())));
            assert_eq!(
                workspace.memo_input.read(cx).value().as_str(),
                "unsaved memo"
            );
            assert!(workspace.error_message.is_some());
            workspace.discard_unsaved_and_close(window);
        })
        .unwrap();
}

#[gpui::test]
fn clicking_selected_task_keeps_draft_until_switch_is_saved(cx: &mut TestAppContext) {
    let (_directory, window, [a, b]) = workspace(cx);
    window
        .update(cx, |workspace, window, cx| {
            workspace.select_task(a, window, cx);
            workspace
                .title_input
                .update(cx, |input, cx| input.set_value("draft", window, cx));
            workspace.schedule_title_save("draft".into(), cx);
            workspace.select_task(a, window, cx);
            assert_eq!(workspace.title_input.read(cx).value().as_str(), "draft");
            workspace.select_task(b, window, cx);
            assert_eq!(workspace.selected_task, Some(b));
            assert_eq!(
                workspace
                    .worker
                    .load()
                    .unwrap()
                    .tasks
                    .iter()
                    .find(|task| task.id == a)
                    .unwrap()
                    .title,
                "draft"
            );
            assert!(workspace.pending_title.is_none());
            window.remove_window();
        })
        .unwrap();
}

#[gpui::test]
fn duplicate_saves_source_draft_and_initializes_copy_editor(cx: &mut TestAppContext) {
    let (_directory, window, [a, _]) = workspace(cx);
    window
        .update(cx, |workspace, window, cx| {
            workspace.select_task(a, window, cx);
            workspace
                .title_input
                .update(cx, |input, cx| input.set_value("draft", window, cx));
            workspace.schedule_title_save("draft".into(), cx);
            workspace.schedule_memo_save("draft memo".into(), cx);
            workspace.duplicate_task(a, window, cx);
            let copy = workspace.selected_task().unwrap();
            assert_ne!(copy.id, a);
            assert_eq!(copy.title, "draft のコピー");
            assert_eq!(copy.memo, "draft memo");
            assert_eq!(workspace.title_input.read(cx).value().as_str(), copy.title);
            assert_eq!(workspace.memo_input.read(cx).value().as_str(), copy.memo);
            assert!(workspace.pending_title.is_none());
            assert!(workspace.pending_memo.is_none());
            let snapshot = workspace.worker.load().unwrap();
            let source = snapshot.tasks.iter().find(|task| task.id == a).unwrap();
            assert_eq!(source.title, "draft");
            assert_eq!(source.memo, "draft memo");
            window.remove_window();
        })
        .unwrap();
}

#[gpui::test]
fn duplicate_keeps_open_new_task_draft(cx: &mut TestAppContext) {
    let (_directory, window, [a, _]) = workspace(cx);
    window
        .update(cx, |workspace, window, cx| {
            workspace.open_new_task_form(window, cx);
            workspace.title_input.update(cx, |input, cx| {
                input.set_value("unfinished new task", window, cx)
            });
            workspace
                .memo_input
                .update(cx, |input, cx| input.set_value("new memo", window, cx));
            workspace.duplicate_task(a, window, cx);
            assert!(workspace.new_task_draft.is_some());
            assert!(workspace.selected_task.is_none());
            assert_eq!(
                workspace.title_input.read(cx).value().as_str(),
                "unfinished new task"
            );
            assert_eq!(workspace.memo_input.read(cx).value().as_str(), "new memo");
            assert_eq!(workspace.worker.load().unwrap().tasks.len(), 3);
            assert!(workspace.create_task(cx));
            assert!(
                workspace
                    .worker
                    .load()
                    .unwrap()
                    .tasks
                    .iter()
                    .any(|task| task.title == "unfinished new task" && task.memo == "new memo")
            );
            window.remove_window();
        })
        .unwrap();
}

#[gpui::test]
fn trash_keeps_invalid_title_editable_until_it_is_corrected(cx: &mut TestAppContext) {
    let (_directory, window, [a, _]) = workspace(cx);
    window
        .update(cx, |workspace, window, cx| {
            workspace.select_task(a, window, cx);
            workspace
                .title_input
                .update(cx, |input, cx| input.set_value("", window, cx));
            workspace.schedule_title_save("".into(), cx);
            workspace.move_to_trash(a, cx);
            assert_eq!(workspace.selected_task, Some(a));
            assert!(workspace.selected_task().unwrap().deleted_at.is_none());
            assert!(
                workspace
                    .worker
                    .load()
                    .unwrap()
                    .tasks
                    .iter()
                    .find(|task| task.id == a)
                    .unwrap()
                    .deleted_at
                    .is_none()
            );
            assert_eq!(workspace.pending_title, Some((a, String::new())));
            workspace.schedule_title_save("corrected".into(), cx);
            workspace.move_to_trash(a, cx);
            assert!(workspace.selected_task.is_none());
            assert!(workspace.pending_title.is_none());
            let snapshot = workspace.worker.load().unwrap();
            let task = snapshot.tasks.iter().find(|task| task.id == a).unwrap();
            assert_eq!(task.title, "corrected");
            assert!(task.deleted_at.is_some());
            assert!(workspace.should_close(cx));
            window.remove_window();
        })
        .unwrap();
}
