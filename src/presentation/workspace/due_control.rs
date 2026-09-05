use gpui::{
    AnyElement, Context, Corner, Focusable as _, InteractiveElement as _, IntoElement, MouseButton,
    ParentElement as _, Styled as _, Window, anchored, canvas, deferred, div, point,
    prelude::FluentBuilder as _, px,
};
use gpui_component::{
    Disableable as _, Icon, IconName, Selectable as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    calendar::{Calendar, Date as PickerDate},
    input::{Enter, Escape, Input},
    scroll::ScrollableElement as _,
};

use super::{Workspace, due::*, section_label, theme};

impl Workspace {
    fn update_selected_due(&mut self, value: &str, cx: &mut Context<Self>) {
        match parse_due(value) {
            Ok(due) => {
                self.update_selected_task(cx, |task, now| {
                    if task.due != due {
                        task.due = due;
                        task.touch(now);
                    }
                });
                self.error_message = None;
                cx.notify();
            }
            Err(message) => {
                self.error_message = Some(message);
                cx.notify();
            }
        }
    }

    pub(super) fn update_due_from_input(
        &mut self,
        value: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.due_input_error = parse_due(value).err();
        self.update_selected_due(value, cx);
        if let Ok(due) = parse_due(value) {
            let picker_date = picker_date_from_due(&due);
            self.due_calendar.update(cx, |state, cx| {
                state.set_date(picker_date, window, cx);
            });
        }
    }

    fn apply_due_time_selection(
        &mut self,
        time: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let current = self.due_input.read(cx).value().to_string();
        match due_input_with_time(&current, time) {
            Ok(value) => {
                self.due_input.update(cx, |state, cx| {
                    state.set_value(value.clone(), window, cx);
                });
                if self.selected_task.is_some() {
                    self.update_selected_due(&value, cx);
                } else {
                    self.error_message = None;
                    cx.notify();
                }
            }
            Err(message) => {
                self.error_message = Some(message);
                cx.notify();
            }
        }
    }

    pub(super) fn clear_due(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.due_input_error = None;
        self.due_input
            .update(cx, |state, cx| state.set_value("", window, cx));
        self.due_calendar.update(cx, |state, cx| {
            state.set_date(PickerDate::Single(None), window, cx);
        });
        if self.selected_task.is_some() {
            self.update_selected_due("", cx);
        } else {
            self.error_message = None;
            cx.notify();
        }
    }

    pub(super) fn dismiss_due_popover(&mut self, cx: &mut Context<Self>) {
        if self.due_popover_open {
            self.due_popover_open = false;
            self.show_due_times = false;
            cx.notify();
        }
    }

    fn open_due_popover(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let value = self.due_input.read(cx).value().to_string();
        self.sync_due_picker_from_input(&value, window, cx);
        // Opening the calendar must not steal the caret or select/replace the text.
        self.due_popover_open = true;
        cx.notify();
    }

    pub(super) fn sync_due_picker_from_input(
        &mut self,
        value: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(previous_error) = self.due_input_error.take() {
            let error = parse_due(value).err();
            // Only replace the footer message if it still belongs to this field.
            // A later storage or title error must survive a valid due correction.
            if self.error_message.as_ref() == Some(&previous_error) {
                self.error_message = error.clone();
            }
            self.due_input_error = error;
        }
        // Partial keyboard input is not an error and must not be written to the task.
        if let Ok(due) = parse_due(value) {
            let date = picker_date_from_due(&due);
            if self.due_calendar.read(cx).date() != date {
                self.due_calendar.update(cx, |calendar, cx| {
                    calendar.set_date(date, window, cx);
                });
            }
        }
        cx.notify();
    }

    pub(super) fn select_due_date(
        &mut self,
        date: PickerDate,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let current = self.due_input.read(cx).value().to_string();
        let value = picker_due_input_value(date, &current);
        self.due_input.update(cx, |state, cx| {
            state.set_value(value.clone(), window, cx);
        });
        self.update_due_from_input(&value, window, cx);
        self.due_input.read(cx).focus_handle(cx).focus(window);
    }

    pub(super) fn render_due_control(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let entity = cx.entity();
        div()
            .debug_selector(|| "due-control".to_owned())
            .track_focus(&self.due_focus)
            // InputState emits PressEnter, then propagates the action. Consume it here
            // so it cannot insert a newline or activate an unrelated default action.
            .on_action(|_: &Enter, _, _| {})
            .on_action(cx.listener(|this, _: &Escape, _, cx| {
                if this.due_popover_open {
                    this.dismiss_due_popover(cx);
                } else {
                    cx.propagate();
                }
            }))
            .flex()
            .flex_col()
            .gap_2()
            .child(section_label("納期（任意）"))
            .child(
                div()
                    .id("due-input-control")
                    .debug_selector(|| "due-input-control".to_owned())
                    .relative()
                    .w_full()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, window, cx| {
                            this.open_due_popover(window, cx);
                        }),
                    )
                    .child(
                        Input::new(&self.due_input)
                            .w_full()
                            .selected(self.due_popover_open)
                            .suffix(Icon::new(IconName::Calendar).small()),
                    )
                    .child(
                        canvas(
                            move |bounds, _, cx| {
                                entity.update(cx, |this, _| this.due_input_bounds = Some(bounds));
                            },
                            |_, _, _, _| {},
                        )
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full(),
                    ),
            )
            .when_some(self.due_input_error.clone(), |this, error| {
                // Keep validation local so title/memo autosave cannot clear this error.
                this.child(
                    div()
                        .debug_selector(|| "due-input-error".to_owned())
                        .text_size(px(12.))
                        .text_color(theme::DANGER)
                        .child(error),
                )
            })
            .when(self.due_popover_open, |this| {
                this.child(self.render_due_popover(window, cx))
            })
            .into_any_element()
    }

    fn render_due_times(&self, has_date: bool, cx: &mut Context<Self>) -> AnyElement {
        let selected_time = parse_due(self.due_input.read(cx).value().as_str())
            .ok()
            .as_ref()
            .and_then(due_time_value);
        let mut options = due_time_options();
        if let Some(time) = &selected_time
            && let Err(index) = options.binary_search(time)
        {
            options.insert(index, time.clone());
        }
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(section_label("時刻（任意）"))
            .child(
                Button::new("toggle-due-times")
                    .debug_selector(|| "due-time-control".to_owned())
                    .w_full()
                    .small()
                    .disabled(!has_date)
                    .label(
                        selected_time
                            .clone()
                            .unwrap_or_else(|| "時刻なし".to_owned()),
                    )
                    .icon(if self.show_due_times {
                        IconName::ChevronUp
                    } else {
                        IconName::ChevronDown
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.show_due_times = !this.show_due_times;
                        cx.notify();
                    })),
            )
            .when(self.show_due_times && has_date, |this| {
                this.child(
                    // Keep choices inline: GPUI cannot nest deferred popups safely.
                    div()
                        .id("due-time-options")
                        .debug_selector(|| "due-time-options".to_owned())
                        .h(px(120.))
                        .overflow_y_scrollbar()
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap_2()
                                .child(
                                    Button::new("remove-due-time")
                                        .small()
                                        .ghost()
                                        .w_full()
                                        .debug_selector(|| "due-remove-time".to_owned())
                                        .label("時刻を外す")
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.apply_due_time_selection(None, window, cx);
                                            this.show_due_times = false;
                                            cx.notify();
                                        })),
                                )
                                .child(div().grid().grid_cols(4).gap_1().children(
                                    options.into_iter().map(|time| {
                                        let selected = selected_time.as_ref() == Some(&time);
                                        Button::new(gpui::SharedString::from(format!(
                                            "due-time-{time}"
                                        )))
                                        .debug_selector({
                                            let time = time.clone();
                                            move || format!("due-time-{time}")
                                        })
                                        .small()
                                        .ghost()
                                        .label(time.clone())
                                        .selected(selected)
                                        .on_click(
                                            cx.listener(move |this, _, window, cx| {
                                                this.apply_due_time_selection(
                                                    Some(&time),
                                                    window,
                                                    cx,
                                                );
                                                this.show_due_times = false;
                                                cx.notify();
                                            }),
                                        )
                                    }),
                                )),
                        ),
                )
            })
            .into_any_element()
    }

    fn render_due_popover(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(bounds) = self.due_input_bounds else {
            return div().into_any_element();
        };
        let viewport = window.viewport_size();
        let below = (viewport.height - bounds.bottom() - px(12.)).max(px(0.));
        let above = (bounds.top() - px(12.)).max(px(0.));
        let show_above = below < px(380.) && above > below;
        let (corner, position, available_height) = if show_above {
            (
                Corner::BottomLeft,
                point(bounds.left(), bounds.top() - px(4.)),
                above,
            )
        } else {
            (
                Corner::TopLeft,
                point(bounds.left(), bounds.bottom() + px(4.)),
                below,
            )
        };
        let has_date = parse_due(self.due_input.read(cx).value().as_str())
            .ok()
            .and_then(|due| due_date(&due))
            .is_some();
        deferred(
            anchored()
                .anchor(corner)
                .position(position)
                .snap_to_window_with_margin(px(8.))
                .child(
                    div()
                        .id("due-popover")
                        .debug_selector(|| "due-popover".to_owned())
                        .occlude()
                        .w(px(288.).min(viewport.width - px(16.)))
                        .max_h(available_height)
                        .overflow_y_scrollbar()
                        .bg(theme::SURFACE)
                        .border_1()
                        .border_color(theme::BORDER)
                        .rounded_lg()
                        .shadow_lg()
                        .on_mouse_down_out(cx.listener(
                            |this, event: &gpui::MouseDownEvent, _, cx| {
                                // Repositioning the caret inside the field is not an outside click.
                                if !this
                                    .due_input_bounds
                                    .is_some_and(|b| b.contains(&event.position))
                                {
                                    this.dismiss_due_popover(cx);
                                }
                            },
                        ))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .flex_shrink_0()
                                .p_3()
                                .gap_3()
                                .child(
                                    div().debug_selector(|| "due-calendar".to_owned()).child(
                                        Calendar::new(&self.due_calendar)
                                            .small()
                                            .number_of_months(1)
                                            .w_full()
                                            .border_0()
                                            .p_0(),
                                    ),
                                )
                                .child(self.render_due_times(has_date, cx))
                                .child(
                                    div()
                                        .text_size(px(12.))
                                        .text_color(theme::MUTED)
                                        .child("直接入力: YYYY-MM-DD / YYYY-MM-DD HH:MM"),
                                )
                                .child(
                                    div()
                                        .grid()
                                        .grid_cols(2)
                                        .gap_2()
                                        .child(
                                            Button::new("clear-due")
                                                .debug_selector(|| "due-clear-control".to_owned())
                                                .small()
                                                .ghost()
                                                .label("未定に戻す")
                                                .on_click(cx.listener(|this, _, window, cx| {
                                                    this.clear_due(window, cx);
                                                    this.dismiss_due_popover(cx);
                                                    this.due_input
                                                        .read(cx)
                                                        .focus_handle(cx)
                                                        .focus(window);
                                                })),
                                        )
                                        .child(
                                            Button::new("confirm-due")
                                                .debug_selector(|| "due-confirm-control".to_owned())
                                                .small()
                                                .primary()
                                                .label("確定")
                                                .on_click(cx.listener(|this, _, window, cx| {
                                                    let value =
                                                        this.due_input.read(cx).value().to_string();
                                                    this.update_due_from_input(&value, window, cx);
                                                    if parse_due(&value).is_ok() {
                                                        this.dismiss_due_popover(cx);
                                                    }
                                                    this.due_input
                                                        .read(cx)
                                                        .focus_handle(cx)
                                                        .focus(window);
                                                })),
                                        ),
                                ),
                        ),
                ),
        )
        .with_priority(1)
        .into_any_element()
    }
}

#[cfg(test)]
mod tests {
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
}
