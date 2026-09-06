use std::time::Duration as StdDuration;

use gpui::{
    AnyElement, Context, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _,
    SharedString, Styled as _, Timer, Window, div, px,
};
use gpui_component::{
    Selectable as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    input::Input,
    progress::Progress,
    scroll::ScrollableElement as _,
};
use time::OffsetDateTime;

use crate::domain::{Priority, Task, TaskId, TaskStatus};

use super::theme;

use super::due::{format_due_input, parse_due, picker_date_from_due};
use super::{PendingConfirmation, SmartView, Workspace, labeled_input, section_label};
// The scroll viewport owns a non-shrinking form so short windows cannot compress controls.
impl Workspace {
    fn render_new_task_detail(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(draft) = self.new_task_draft.clone() else {
            return div().into_any_element();
        };
        div()
            .id("new-task-detail-panel")
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .h_full()
            .border_l_1()
            .border_color(theme::BORDER)
            .bg(theme::SURFACE)
            .overflow_x_hidden()
            .overflow_y_scrollbar()
            .child(
                div()
                    .debug_selector(|| "task-editor-content".to_owned())
                    .flex()
                    .flex_col()
                    .flex_shrink_0()
                    .w_full()
                    .min_w_0()
                    .p_4()
                    .gap_4()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .text_size(px(18.0))
                                    .font_weight(FontWeight::BOLD)
                                    .child("新規タスク"),
                            )
                            .child(self.small_action_button(
                                "close-new-task",
                                "閉じる",
                                cx,
                                |this, _, cx| this.close_task_form(cx),
                            )),
                    )
                    .child(labeled_input("タイトル", Input::new(&self.title_input)))
                    .child(labeled_input("メモ", self.render_memo_input()))
                    .child(self.render_due_control(window, cx))
                    .child(section_label("状態"))
                    .child(
                        div().flex().flex_wrap().gap_2().children(
                            TaskStatus::ALL
                                .into_iter()
                                .filter(|status| *status != TaskStatus::Archived)
                                .map(|status| {
                                    let entity = cx.entity();
                                    Button::new(SharedString::from(format!(
                                        "new-status-{}",
                                        status.as_str()
                                    )))
                                    .small()
                                    .label(status.label())
                                    .selected(draft.status == status)
                                    .on_click(
                                        move |_, window, cx| {
                                            entity.update(cx, |this, cx| {
                                                this.set_new_task_status(status, window, cx);
                                            });
                                        },
                                    )
                                }),
                        ),
                    )
                    .child(section_label("優先度"))
                    .child(div().flex().flex_wrap().gap_2().children(
                        Priority::ALL.into_iter().map(|priority| {
                            let entity = cx.entity();
                            Button::new(SharedString::from(format!(
                                "new-priority-{}",
                                priority.as_str()
                            )))
                            .small()
                            .label(priority.label())
                            .selected(draft.priority == priority)
                            .on_click(move |_, _, cx| {
                                entity.update(cx, |this, cx| {
                                    this.set_new_task_priority(priority, cx);
                                });
                            })
                        }),
                    ))
                    .child(section_label("進捗"))
                    .child(Progress::new().value(f32::from(draft.progress)))
                    .child(labeled_input(
                        "直接入力（0〜100）",
                        Input::new(&self.progress_input),
                    ))
                    .child(div().flex().flex_wrap().gap_2().children(
                        [0, 25, 50, 75, 100].into_iter().map(|progress| {
                            let entity = cx.entity();
                            Button::new(SharedString::from(format!("new-progress-{progress}")))
                                .small()
                                .label(format!("{progress}%"))
                                .selected(draft.progress == progress)
                                .on_click(move |_, window, cx| {
                                    entity.update(cx, |this, cx| {
                                        this.set_new_task_progress(progress, window, cx);
                                    });
                                })
                        }),
                    ))
                    .child({
                        let entity = cx.entity();
                        Button::new("save-new-task")
                            .primary()
                            .label("タスクを保存")
                            .on_click(move |_, _, cx| {
                                entity.update(cx, |this, cx| {
                                    this.create_task(cx);
                                });
                            })
                    }),
            )
            .into_any_element()
    }

    pub(super) fn render_detail(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        if self.new_task_draft.is_some() {
            return self.render_new_task_detail(window, cx);
        }
        let Some(task) = self.selected_task().cloned() else {
            return div()
                .flex()
                .items_center()
                .justify_center()
                .flex_1()
                .min_w_0()
                .h_full()
                .border_l_1()
                .border_color(theme::BORDER)
                .bg(theme::SURFACE)
                .text_color(theme::MUTED)
                .child("タスクを選択してください")
                .into_any_element();
        };
        let id = task.id;
        div()
            .id("task-detail-panel")
            .debug_selector(|| "task-detail-panel".to_owned())
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .h_full()
            .border_l_1()
            .border_color(theme::BORDER)
            .bg(theme::SURFACE)
            .overflow_x_hidden()
            .overflow_y_scrollbar()
            .child(
                div()
                    .debug_selector(|| "task-editor-content".to_owned())
                    .flex()
                    .flex_col()
                    .flex_shrink_0()
                    .w_full()
                    .min_w_0()
                    .p_4()
                    .gap_4()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .text_size(px(18.0))
                                    .font_weight(FontWeight::BOLD)
                                    .child("タスク詳細"),
                            )
                            .child(self.small_action_button(
                                "close-detail",
                                "閉じる",
                                cx,
                                |this, _, cx| this.close_task_form(cx),
                            )),
                    )
                    .child(labeled_input("タイトル", Input::new(&self.title_input)))
                    .child(labeled_input("メモ", self.render_memo_input()))
                    .child(self.render_due_control(window, cx))
                    .child(section_label("状態"))
                    .child(
                        div().flex().flex_wrap().gap_2().children(
                            TaskStatus::ALL
                                .into_iter()
                                .filter(|status| *status != TaskStatus::Archived)
                                .map(|status| {
                                    let entity = cx.entity();
                                    Button::new(SharedString::from(format!(
                                        "status-{}",
                                        status.as_str()
                                    )))
                                    .small()
                                    .label(status.label())
                                    .selected(task.status == status)
                                    .on_click(
                                        move |_, _, cx| {
                                            entity.update(cx, |this, cx| {
                                                this.set_task_status(id, status, cx);
                                            });
                                        },
                                    )
                                }),
                        ),
                    )
                    .child(section_label("優先度"))
                    .child(div().flex().flex_wrap().gap_2().children(
                        Priority::ALL.into_iter().map(|priority| {
                            let entity = cx.entity();
                            Button::new(SharedString::from(format!(
                                "priority-{}",
                                priority.as_str()
                            )))
                            .small()
                            .label(priority.label())
                            .selected(task.priority == priority)
                            .on_click(move |_, _, cx| {
                                entity.update(cx, |this, cx| {
                                    this.set_task_priority(id, priority, cx);
                                });
                            })
                        }),
                    ))
                    .child(section_label("進捗"))
                    .child(Progress::new().value(f32::from(task.progress)))
                    .child(labeled_input(
                        "直接入力（0〜100）",
                        Input::new(&self.progress_input),
                    ))
                    .child(
                        div()
                            .debug_selector(|| "task-progress-presets".to_owned())
                            .flex()
                            .flex_wrap()
                            .gap_2()
                            .children([0, 25, 50, 75, 100].into_iter().map(|progress| {
                                let entity = cx.entity();
                                Button::new(SharedString::from(format!("progress-{progress}")))
                                    .small()
                                    .label(format!("{progress}%"))
                                    .selected(task.progress == progress)
                                    .on_click(move |_, _, cx| {
                                        entity.update(cx, |this, cx| {
                                            this.set_task_progress(id, progress, cx);
                                        });
                                    })
                            })),
                    )
                    .child(self.render_task_actions(id, cx)),
            )
            .into_any_element()
    }

    fn render_task_actions(&self, id: TaskId, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .flex_shrink_0()
            .w_full()
            .mt_2()
            .pt_4()
            .border_t_1()
            .border_color(theme::BORDER)
            .gap_3()
            .child({
                let entity = cx.entity();
                Button::new("save-task-detail")
                    .debug_selector(|| "task-save-button".to_owned())
                    .primary()
                    .w_full()
                    .flex_shrink_0()
                    .label("変更を保存")
                    .on_click(move |_, _, cx| {
                        entity.update(cx, |this, cx| {
                            this.save_and_close_selected_task(cx);
                        });
                    })
            })
            .child(
                div()
                    .grid()
                    .grid_cols(2)
                    .gap_3()
                    .w_full()
                    .flex_shrink_0()
                    .child({
                        let entity = cx.entity();
                        Button::new("archive-task")
                            .debug_selector(|| "task-archive-button".to_owned())
                            .w_full()
                            .label("アーカイブ")
                            .on_click(move |_, _, cx| {
                                entity.update(cx, |this, cx| {
                                    this.set_task_status(id, TaskStatus::Archived, cx);
                                });
                            })
                    })
                    .child({
                        let entity = cx.entity();
                        Button::new("trash-task")
                            .debug_selector(|| "task-trash-button".to_owned())
                            .danger()
                            .w_full()
                            .label("ゴミ箱へ")
                            .on_click(move |_, _, cx| {
                                entity.update(cx, |this, cx| this.move_to_trash(id, cx));
                            })
                    }),
            )
            .into_any_element()
    }
}

impl Workspace {
    pub(super) fn select_task(&mut self, id: TaskId, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_task == Some(id) {
            self.selection_anchor = Some(id);
            cx.notify();
            return;
        }
        if !self.flush_pending_edits(cx) {
            return;
        }
        self.selected_task = Some(id);
        self.new_task_draft = None;
        self.selection_anchor = Some(id);
        self.sync_detail_inputs(window, cx);
        cx.notify();
    }

    #[must_use]
    pub(super) fn flush_pending_edits(&mut self, cx: &mut Context<Self>) -> bool {
        self.title_revision = self.title_revision.wrapping_add(1);
        self.memo_revision = self.memo_revision.wrapping_add(1);
        if let Err(message) = self.persist_pending_edits() {
            self.set_pending_edit_error(message);
            cx.notify();
            return false;
        }
        cx.notify();
        true
    }

    pub(super) fn persist_pending_edits(&mut self) -> Result<(), String> {
        if self.pending_title.is_none() && self.pending_memo.is_none() {
            return Ok(());
        }
        let mut tasks = self.tasks.clone();
        apply_pending_edits(
            &mut tasks,
            self.pending_title.as_ref(),
            self.pending_memo.as_ref(),
            OffsetDateTime::now_utc(),
        )?;
        let history = self
            .tasks
            .iter()
            .zip(&tasks)
            .filter(|(before, after)| before != after)
            .map(|(before, after)| (Some(before.clone()), Some(after.clone())))
            .collect::<Vec<_>>();
        if !history.is_empty() {
            self.worker
                .save_tasks(
                    history
                        .iter()
                        .filter_map(|(_, after)| after.clone())
                        .collect(),
                )
                .map_err(|error| error.to_string())?;
        }
        self.tasks = tasks;
        self.pending_title = None;
        self.pending_memo = None;
        self.push_task_history(history);
        self.error_message = None;
        self.status_message = "保存済み".to_owned();
        Ok(())
    }

    pub(super) fn set_pending_edit_error(&mut self, message: String) {
        while self.worker.take_error().is_some() {}
        self.error_message = Some(message);
        self.status_message = if self.worker.is_read_only() {
            "読み取り専用".to_owned()
        } else {
            "保存失敗".to_owned()
        };
    }

    pub(in crate::presentation) fn should_close(&mut self, cx: &mut Context<Self>) -> bool {
        if self.allow_close || self.close_save_completed {
            return true;
        }
        match self.persist_before_close() {
            Ok(()) => true,
            Err(message) => {
                while self.worker.take_error().is_some() {}
                self.error_message = Some(format!("終了前の保存に失敗しました: {message}"));
                self.status_message = "保存失敗 — 終了を保留中".to_owned();
                self.pending_confirmation = Some(PendingConfirmation::CloseSaveFailed);
                cx.notify();
                false
            }
        }
    }

    pub(super) fn persist_before_close(&mut self) -> Result<(), String> {
        self.persist_pending_edits()?;
        self.worker.flush().map_err(|error| error.to_string())?;
        self.update_persisted_settings_fields();
        self.settings
            .save(&self.paths.settings)
            .map_err(|error| error.to_string())?;
        self.close_save_completed = true;
        self.status_message = "保存済み".to_owned();
        Ok(())
    }

    pub(super) fn retry_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.pending_confirmation = None;
        if self.should_close(cx) {
            self.allow_close = true;
            window.remove_window();
        }
    }

    pub(super) fn discard_unsaved_and_close(&mut self, window: &mut Window) {
        self.pending_title = None;
        self.pending_memo = None;
        self.pending_confirmation = None;
        self.discard_unsaved_on_close = true;
        self.allow_close = true;
        window.remove_window();
    }

    pub(super) fn sync_management_inputs(
        &mut self,
        view: SmartView,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let name = match view {
            SmartView::Saved(id) => self
                .saved_views
                .iter()
                .find(|view| view.id == id)
                .map(|view| view.name.clone())
                .unwrap_or_default(),
            _ => String::new(),
        };
        self.view_name_input.update(cx, |state, cx| {
            state.set_value(name, window, cx);
        });
    }

    pub(super) fn sync_detail_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.dismiss_due_popover(cx);
        self.due_input_error = None;
        let Some(task) = self.selected_task().cloned() else {
            return;
        };
        let picker_date = picker_date_from_due(&task.due);
        self.title_input.update(cx, |state, cx| {
            state.set_value(task.title, window, cx);
        });
        self.memo_input.update(cx, |state, cx| {
            state.set_value(task.memo, window, cx);
        });
        self.due_input.update(cx, |state, cx| {
            state.set_value(format_due_input(&task.due), window, cx);
        });
        self.due_calendar.update(cx, |state, cx| {
            state.set_date(picker_date, window, cx);
        });
        self.progress_input.update(cx, |state, cx| {
            state.set_value(task.progress.to_string(), window, cx);
        });
    }

    pub(super) fn schedule_title_save(&mut self, title: String, cx: &mut Context<Self>) {
        let Some(id) = self.selected_task else {
            return;
        };
        let invalid = title.trim().is_empty() || title.chars().count() > 500;
        self.title_revision = self.title_revision.wrapping_add(1);
        let revision = self.title_revision;
        self.pending_title = Some((id, title));
        if invalid {
            self.error_message = Some("タイトルは1〜500文字で入力してください".to_owned());
            self.status_message = "入力エラー".to_owned();
            cx.notify();
            return;
        }
        self.status_message = "編集中…".to_owned();
        cx.spawn(async move |this, cx| {
            Timer::after(StdDuration::from_millis(400)).await;
            if let Some(this) = this.upgrade() {
                this.update(cx, |this, cx| {
                    if this.title_revision != revision {
                        return;
                    }
                    if this.pending_title.is_none() {
                        return;
                    }
                    if let Err(message) = this.persist_pending_edits() {
                        this.set_pending_edit_error(message);
                    }
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
        cx.notify();
    }

    pub(super) fn schedule_memo_save(&mut self, memo: String, cx: &mut Context<Self>) {
        let Some(id) = self.selected_task else {
            return;
        };
        self.memo_revision = self.memo_revision.wrapping_add(1);
        let revision = self.memo_revision;
        self.pending_memo = Some((id, memo));
        self.status_message = "編集中…".to_owned();
        cx.spawn(async move |this, cx| {
            Timer::after(StdDuration::from_millis(400)).await;
            if let Some(this) = this.upgrade() {
                this.update(cx, |this, cx| {
                    if this.memo_revision != revision {
                        return;
                    }
                    if this.pending_memo.is_none() {
                        return;
                    }
                    if let Err(message) = this.persist_pending_edits() {
                        this.set_pending_edit_error(message);
                    }
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
        cx.notify();
    }

    pub(super) fn save_selected_task_form(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(id) = self.selected_task else {
            return false;
        };
        let Some(before) = self.tasks.iter().find(|task| task.id == id).cloned() else {
            return false;
        };
        let mut after = before.clone();
        if let Err(error) = after.set_title(self.title_input.read(cx).value().to_string()) {
            self.error_message = Some(error.to_string());
            cx.notify();
            return false;
        }
        after.memo = self.memo_input.read(cx).value().to_string();
        after.due = match parse_due(self.due_input.read(cx).value().as_str()) {
            Ok(due) => {
                self.due_input_error = None;
                due
            }
            Err(message) => {
                self.due_input_error = Some(message.clone());
                self.error_message = Some(message);
                cx.notify();
                return false;
            }
        };
        let progress = match self.progress_input.read(cx).value().trim().parse::<u8>() {
            Ok(progress) if progress <= 100 => progress,
            _ => {
                self.error_message = Some("進捗は0〜100の整数で入力してください".to_owned());
                cx.notify();
                return false;
            }
        };
        let _ = after.set_progress(progress);

        if after != before {
            after.touch(OffsetDateTime::now_utc());
            if let Err(error) = self.worker.save_task(after.clone()) {
                self.set_error(error);
                cx.notify();
                return false;
            }
            if let Some(task) = self.tasks.iter_mut().find(|task| task.id == id) {
                *task = after.clone();
            }
            self.push_task_history(vec![(Some(before), Some(after))]);
        }

        self.title_revision = self.title_revision.wrapping_add(1);
        self.memo_revision = self.memo_revision.wrapping_add(1);
        self.pending_title = None;
        self.pending_memo = None;
        self.error_message = None;
        self.status_message = "タスクの変更を保存しました".to_owned();
        cx.notify();
        true
    }

    pub(super) fn save_and_close_selected_task(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.save_selected_task_form(cx) {
            return false;
        }
        self.selected_task = None;
        self.new_task_draft = None;
        cx.notify();
        true
    }

    fn render_memo_input(&self) -> AnyElement {
        div()
            .debug_selector(|| "task-memo-input".to_owned())
            .w_full()
            .h(px(160.0))
            .flex_shrink_0()
            .overflow_hidden()
            .child(Input::new(&self.memo_input).h_full())
            .into_any_element()
    }
}

pub(super) fn apply_pending_edits(
    tasks: &mut [Task],
    pending_title: Option<&(TaskId, String)>,
    pending_memo: Option<&(TaskId, String)>,
    now: OffsetDateTime,
) -> Result<bool, String> {
    let mut changed = false;
    if let Some((id, title)) = pending_title
        && let Some(task) = tasks.iter_mut().find(|task| task.id == *id)
    {
        task.set_title(title.clone())
            .map_err(|error| error.to_string())?;
        task.touch(now);
        changed = true;
    }
    if let Some((id, memo)) = pending_memo
        && let Some(task) = tasks.iter_mut().find(|task| task.id == *id)
    {
        task.memo = memo.clone();
        task.touch(now);
        changed = true;
    }
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        application::TaskApplication,
        infrastructure::{AppPaths, AppSettings, InstanceLock},
    };
    use gpui::{TestAppContext, WindowHandle};

    fn workspace(
        cx: &mut TestAppContext,
    ) -> (tempfile::TempDir, WindowHandle<Workspace>, [TaskId; 2]) {
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
}
