use super::*;
use crate::{
    application::TaskApplication,
    domain::{Due, Task},
    infrastructure::{AppPaths, AppSettings, InstanceLock},
};
use gpui::{AppContext as _, Modifiers, TestAppContext, VisualTestContext, WindowHandle, size};
use time::OffsetDateTime;

fn workspace(
    cx: &mut TestAppContext,
) -> (
    tempfile::TempDir,
    WindowHandle<gpui_component::Root>,
    gpui::Entity<Workspace>,
) {
    let directory = tempfile::tempdir().unwrap();
    let paths = AppPaths::resolve(Some(directory.path())).unwrap();
    let lock = InstanceLock::acquire(&paths.lock).unwrap();
    let worker = TaskApplication::start(&paths.database).unwrap();
    let task = Task::new("納期入力のテスト", OffsetDateTime::now_utc()).unwrap();
    let id = task.id;
    worker.save_task(task).unwrap();
    let snapshot = worker.load().unwrap();
    cx.update(gpui_component::init);
    let mut entity = None;
    let window = cx.add_window(|window, cx| {
        let workspace = cx.new(|cx| {
            let mut workspace = Workspace::new(
                worker,
                snapshot,
                paths,
                AppSettings {
                    detail_width: 280.,
                    ..AppSettings::default()
                },
                lock,
                false,
                window,
                cx,
            );
            workspace.select_task(id, window, cx);
            workspace
        });
        entity = Some(workspace.clone());
        gpui_component::Root::new(workspace, window, cx)
    });
    (directory, window, entity.unwrap())
}

#[gpui::test]
fn due_popover_keeps_keyboard_input_and_caret_clicks(cx: &mut TestAppContext) {
    let (_directory, window, workspace) = workspace(cx);
    let mut visual = VisualTestContext::from_window(*window, cx);
    visual.simulate_resize(size(px(1200.), px(1000.)));
    visual.run_until_parked();
    let input = visual.debug_bounds("due-input-control").unwrap();
    assert!(visual.debug_bounds("due-popover").is_none());
    visual.simulate_click(input.center(), Modifiers::default());
    visual.simulate_input("2026-09-08 14:37");
    workspace.update_in(&mut visual, |this, window, cx| {
        assert!(this.due_popover_open);
        assert!(this.due_input.read(cx).focus_handle(cx).is_focused(window));
        assert_eq!(this.due_input.read(cx).value().as_str(), "2026-09-08 14:37");
        assert_eq!(
            this.selected_task().unwrap().due,
            Due::None,
            "typing must not save incomplete values"
        );
        assert_eq!(
            this.due_calendar.read(cx).date(),
            picker_date_from_due(&parse_due("2026-09-08").unwrap())
        );
    });
    visual.simulate_click(input.center(), Modifiers::default());
    workspace.update_in(&mut visual, |this, _, cx| {
        assert!(
            this.due_popover_open,
            "caret clicks must not toggle the popover closed"
        );
        assert_eq!(this.due_input.read(cx).value().as_str(), "2026-09-08 14:37");
    });
    visual.simulate_keystrokes("enter");
    workspace.update_in(&mut visual, |this, _, _| {
        assert!(!this.due_popover_open);
        assert_eq!(
            this.selected_task().unwrap().due,
            parse_due("2026-09-08 14:37").unwrap()
        );
    });
    visual.simulate_click(input.center(), Modifiers::default());
    visual.simulate_keystrokes("escape");
    workspace.update_in(&mut visual, |this, _, cx| {
        assert!(!this.due_popover_open);
        assert!(
            this.selected_task.is_some(),
            "Escape must only close the popover"
        );
        assert_eq!(this.due_input.read(cx).value().as_str(), "2026-09-08 14:37");
    });
    visual.update(|window, _| window.remove_window());
}

#[gpui::test]
fn due_popover_fits_short_window_and_outside_click_dismisses(cx: &mut TestAppContext) {
    let (_directory, window, workspace) = workspace(cx);
    let mut visual = VisualTestContext::from_window(*window, cx);
    visual.simulate_resize(size(px(900.), px(600.)));
    visual.run_until_parked();
    let input = visual.debug_bounds("due-input-control").unwrap();
    visual.simulate_click(input.center(), Modifiers::default());
    let popup = visual.debug_bounds("due-popover").unwrap();
    assert!(popup.left() >= px(0.) && popup.right() <= px(900.));
    assert!(popup.top() >= px(0.) && popup.bottom() <= px(600.));
    assert!(
        popup.bottom() <= input.top() || popup.top() >= input.bottom(),
        "the popup must not cover the editable field: popup={popup:?}, input={input:?}"
    );
    visual.simulate_click(point(px(500.), px(560.)), Modifiers::default());
    workspace.update_in(&mut visual, |this, _, _| assert!(!this.due_popover_open));
    visual.update(|window, _| window.remove_window());
}

#[gpui::test]
fn due_popover_preserves_invalid_text_and_clears_via_button(cx: &mut TestAppContext) {
    let (_directory, window, workspace) = workspace(cx);
    let mut visual = VisualTestContext::from_window(*window, cx);
    visual.simulate_resize(size(px(1200.), px(1000.)));
    visual.run_until_parked();
    let input = visual.debug_bounds("due-input-control").unwrap();
    visual.simulate_click(input.center(), Modifiers::default());
    visual.simulate_input("2026-02-30");
    visual.simulate_keystrokes("enter");
    workspace.update_in(&mut visual, |this, _, cx| {
        assert!(this.due_popover_open);
        assert!(this.due_input_error.is_some());
        this.persist_pending_edits().unwrap();
        assert!(
            this.due_input_error.is_some(),
            "unrelated autosave must not clear due validation"
        );
        assert_eq!(this.selected_task().unwrap().due, Due::None);
        assert_eq!(this.due_input.read(cx).value().as_str(), "2026-02-30");
    });
    let clear = visual.debug_bounds("due-clear-control").unwrap();
    visual.simulate_click(clear.center(), Modifiers::default());
    workspace.update_in(&mut visual, |this, _, cx| {
        assert!(!this.due_popover_open);
        assert!(this.error_message.is_none());
        assert!(this.due_input_error.is_none());
        assert!(this.due_input.read(cx).value().is_empty());
        assert_eq!(this.selected_task().unwrap().due, Due::None);
    });
    visual.update(|window, _| window.remove_window());
}

#[gpui::test]
fn due_popover_calendar_and_inline_time_choices_update_the_same_field(cx: &mut TestAppContext) {
    let (_directory, window, workspace) = workspace(cx);
    let mut visual = VisualTestContext::from_window(*window, cx);
    visual.simulate_resize(size(px(1200.), px(1000.)));
    visual.run_until_parked();
    let input = visual.debug_bounds("due-input-control").unwrap();
    visual.simulate_click(input.center(), Modifiers::default());
    visual.simulate_input("2026-09-08 14:37");
    let calendar = visual.debug_bounds("due-calendar").unwrap();
    // September 2026 ends in a week starting Sunday 27. Click that visible cell.
    visual.simulate_click(
        point(calendar.left() + px(14.), calendar.bottom() - px(14.)),
        Modifiers::default(),
    );
    workspace.update_in(&mut visual, |this, _, cx| {
        assert_eq!(this.due_input.read(cx).value().as_str(), "2026-09-27 14:37");
        assert_eq!(
            this.selected_task().unwrap().due,
            parse_due("2026-09-27 14:37").unwrap()
        );
        assert!(this.due_popover_open);
    });
    let time = visual.debug_bounds("due-time-control").unwrap();
    visual.simulate_click(time.center(), Modifiers::default());
    let option = visual.debug_bounds("due-time-00:15").unwrap();
    visual.simulate_click(option.center(), Modifiers::default());
    workspace.update_in(&mut visual, |this, _, cx| {
        assert_eq!(this.due_input.read(cx).value().as_str(), "2026-09-27 00:15");
        assert!(!this.show_due_times);
        assert!(this.due_popover_open);
    });
    let time = visual.debug_bounds("due-time-control").unwrap();
    visual.simulate_click(time.center(), Modifiers::default());
    let remove = visual.debug_bounds("due-remove-time").unwrap();
    visual.simulate_click(remove.center(), Modifiers::default());
    workspace.update_in(&mut visual, |this, _, cx| {
        assert_eq!(this.due_input.read(cx).value().as_str(), "2026-09-27");
        assert_eq!(
            this.selected_task().unwrap().due,
            parse_due("2026-09-27").unwrap()
        );
    });
    visual.update(|window, _| window.remove_window());
}

#[gpui::test]
fn due_popover_new_task_and_task_switch_do_not_leak_state(cx: &mut TestAppContext) {
    let (_directory, window, workspace) = workspace(cx);
    let mut visual = VisualTestContext::from_window(*window, cx);
    visual.simulate_resize(size(px(1200.), px(1000.)));
    let original_id = workspace.update_in(&mut visual, |this, window, cx| {
        let id = this.selected_task.unwrap();
        this.open_new_task_form(window, cx);
        this.title_input
            .update(cx, |state, cx| state.set_value("新しい納期", window, cx));
        id
    });
    visual.run_until_parked();
    let input = visual.debug_bounds("due-input-control").unwrap();
    visual.simulate_click(input.center(), Modifiers::default());
    visual.simulate_input("2026-12-31 23:59");
    visual.simulate_keystrokes("enter");
    workspace.update_in(&mut visual, |this, window, cx| {
        assert!(!this.due_popover_open);
        assert!(this.create_task(cx));
        let task = this.tasks.iter().find(|t| t.title == "新しい納期").unwrap();
        assert_eq!(task.due, parse_due("2026-12-31 23:59").unwrap());
        this.select_task(task.id, window, cx);
        this.open_due_popover(window, cx);
        this.show_due_times = true;
        this.select_task(original_id, window, cx);
        assert!(!this.due_popover_open);
        assert!(!this.show_due_times);
        assert!(this.due_input.read(cx).value().is_empty());
    });
    visual.update(|window, _| window.remove_window());
}

#[gpui::test]
fn due_validation_on_new_task_save_keeps_invalid_input(cx: &mut TestAppContext) {
    check_due_validation_on_task_save(cx, true);
}

#[gpui::test]
fn due_validation_on_existing_task_save_keeps_invalid_input(cx: &mut TestAppContext) {
    check_due_validation_on_task_save(cx, false);
}

fn check_due_validation_on_task_save(cx: &mut TestAppContext, new_task: bool) {
    let (_directory, window, workspace) = workspace(cx);
    let mut visual = VisualTestContext::from_window(*window, cx);
    visual.simulate_resize(size(px(1200.), px(1400.)));
    let initial_tasks = workspace.update_in(&mut visual, |this, window, cx| {
        if new_task {
            this.open_new_task_form(window, cx);
            this.title_input.update(cx, |state, cx| {
                state.set_value("保存時の納期検証", window, cx);
            });
        } else {
            this.update_due_from_input("2026-09-08", window, cx);
        }
        this.due_input.update(cx, |state, cx| {
            state.set_value("2026-02-30", window, cx);
        });
        let initial = this.worker.load().unwrap().tasks;
        assert!(this.due_input_error.is_none());
        // Invoke the save-button path without Enter or the calendar's confirm button.
        assert!(!if new_task {
            this.create_task(cx)
        } else {
            this.save_selected_task_form(cx)
        });
        assert!(this.due_input_error.is_some());
        assert_eq!(this.due_input_error, this.error_message);
        assert_eq!(this.due_input.read(cx).value().as_str(), "2026-02-30");
        assert_eq!(this.worker.load().unwrap().tasks, initial);
        initial
    });
    visual.run_until_parked();
    let input = visual.debug_bounds("due-input-control").unwrap();
    let error = visual.debug_bounds("due-input-error").unwrap();
    assert!(error.top() >= input.bottom());
    workspace.update_in(&mut visual, |this, window, cx| {
        this.due_input.update(cx, |state, cx| {
            state.set_value("2026-02-28", window, cx);
        });
    });
    visual.run_until_parked();
    assert!(visual.debug_bounds("due-input-error").is_none());
    workspace.update_in(&mut visual, |this, _, cx| {
        assert!(this.due_input_error.is_none());
        assert!(
            this.error_message.is_none(),
            "correcting the due field must clear its footer error too"
        );
        assert!(if new_task {
            this.create_task(cx)
        } else {
            this.save_selected_task_form(cx)
        });
        assert!(this.due_input_error.is_none());
        let stored = this.worker.load().unwrap().tasks;
        assert_eq!(stored.len(), initial_tasks.len() + usize::from(new_task));
        assert!(
            stored
                .iter()
                .any(|task| task.due == parse_due("2026-02-28").unwrap())
        );
    });
    visual.update(|window, _| window.remove_window());
}

#[gpui::test]
fn correcting_due_after_enter_clears_only_the_related_global_error(cx: &mut TestAppContext) {
    let (_directory, window, workspace) = workspace(cx);
    window
        .update(cx, |_, window, cx| {
            workspace.update(cx, |this, cx| {
                this.update_due_from_input("2026-02-30", window, cx);
                assert!(this.due_input_error.is_some());
                assert_eq!(this.due_input_error, this.error_message);
                this.sync_due_picker_from_input("2026-02-28", window, cx);
                assert!(this.due_input_error.is_none());
                assert!(this.error_message.is_none());
                assert_eq!(
                    this.selected_task().unwrap().due,
                    Due::None,
                    "valid typing must not implicitly persist the due value"
                );

                this.update_due_from_input("2026-02-30", window, cx);
                this.error_message = Some("保存先への書き込みに失敗しました".to_owned());
                this.sync_due_picker_from_input("2026-02-28", window, cx);
                assert!(this.due_input_error.is_none());
                assert_eq!(
                    this.error_message.as_deref(),
                    Some("保存先への書き込みに失敗しました"),
                    "correcting due input must not dismiss an unrelated error"
                );
            });
            window.remove_window();
        })
        .unwrap();
}
