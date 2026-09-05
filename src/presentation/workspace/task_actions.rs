//! Task mutations, selection, batch operations and their UI feedback.
use gpui::{
    AnyElement, Context, Focusable as _, IntoElement, ParentElement as _, SharedString,
    Styled as _, Window, div, px,
};
use gpui_component::{
    Sizable as _,
    button::{Button, ButtonVariants as _},
    calendar::Date as PickerDate,
    input::Input,
};
use time::OffsetDateTime;

use crate::domain::{Due, Priority, Task, TaskId, TaskStatus, ViewKind};

use super::due::parse_due;
use super::{NewTaskDraft, Workspace};

impl Workspace {
    pub(super) fn open_new_task_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_task.is_some() && !self.flush_pending_edits(cx) {
            return;
        }
        self.dismiss_due_popover(cx);
        self.due_input_error = None;
        self.selected_task = None;
        self.new_task_draft = Some(NewTaskDraft::default());
        self.title_input
            .update(cx, |state, cx| state.set_value("", window, cx));
        self.memo_input
            .update(cx, |state, cx| state.set_value("", window, cx));
        self.due_input
            .update(cx, |state, cx| state.set_value("", window, cx));
        self.due_calendar.update(cx, |state, cx| {
            state.set_date(PickerDate::Single(None), window, cx);
        });
        self.progress_input
            .update(cx, |state, cx| state.set_value("0", window, cx));
        self.error_message = None;
        self.title_input.read(cx).focus_handle(cx).focus(window);
        cx.notify();
    }

    pub(super) fn close_task_form(&mut self, cx: &mut Context<Self>) {
        if self.selected_task.is_some() && !self.flush_pending_edits(cx) {
            return;
        }
        self.dismiss_due_popover(cx);
        self.selected_task = None;
        self.new_task_draft = None;
        cx.notify();
    }

    pub(super) fn create_task(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(draft) = self.new_task_draft.clone() else {
            return false;
        };
        let now = OffsetDateTime::now_utc();
        let title = self.title_input.read(cx).value().to_string();
        let due = match parse_due(self.due_input.read(cx).value().as_str()) {
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
        match Task::new(title, now) {
            Ok(mut task) => {
                task.memo = self.memo_input.read(cx).value().to_string();
                task.set_status(draft.status, now);
                task.priority = draft.priority;
                let _ = task.set_progress(progress);
                task.due = due;
                task.sort_order = self
                    .tasks
                    .iter()
                    .map(|task| task.sort_order)
                    .max()
                    .unwrap_or_default()
                    + 1024;
                if let Err(error) = self.worker.save_task(task.clone()) {
                    self.set_error(error);
                    cx.notify();
                    return false;
                }
                self.push_task_history(vec![(None, Some(task.clone()))]);
                self.tasks.push(task);
                self.new_task_draft = None;
                self.status_message = "新しいタスクを保存しました".to_owned();
                self.error_message = None;
                cx.notify();
                true
            }
            Err(error) => {
                self.error_message = Some(error.to_string());
                cx.notify();
                false
            }
        }
    }

    pub(super) fn set_new_task_status(
        &mut self,
        status: TaskStatus,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(draft) = self.new_task_draft.as_mut() else {
            return;
        };
        if status == TaskStatus::Done {
            draft.progress = 100;
        } else if draft.status == TaskStatus::Done {
            draft.progress = 0;
        }
        draft.status = status;
        let progress = draft.progress.to_string();
        self.progress_input
            .update(cx, |state, cx| state.set_value(progress, window, cx));
        cx.notify();
    }

    pub(super) fn set_new_task_priority(&mut self, priority: Priority, cx: &mut Context<Self>) {
        if let Some(draft) = self.new_task_draft.as_mut() {
            draft.priority = priority;
            cx.notify();
        }
    }

    pub(super) fn set_new_task_progress(
        &mut self,
        progress: u8,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(draft) = self.new_task_draft.as_mut() {
            draft.progress = progress;
            self.progress_input.update(cx, |state, cx| {
                state.set_value(progress.to_string(), window, cx);
            });
            cx.notify();
        }
    }

    pub(super) fn duplicate_task(
        &mut self,
        id: TaskId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.flush_pending_edits(cx) {
            return;
        }
        let Some(source) = self.tasks.iter().find(|task| task.id == id).cloned() else {
            return;
        };
        let now = OffsetDateTime::now_utc();
        let Ok(mut copy) = Task::new(format!("{} のコピー", source.title), now) else {
            return;
        };
        copy.memo = source.memo;
        copy.status = source.status;
        copy.priority = source.priority;
        copy.progress = source.progress;
        copy.due = source.due;
        copy.project_id = source.project_id;
        copy.tag_ids = source.tag_ids;
        copy.sort_order = self
            .tasks
            .iter()
            .map(|task| task.sort_order)
            .max()
            .unwrap_or_default()
            + 1024;
        if let Err(error) = self.worker.save_task(copy.clone()) {
            self.set_error(error);
            return;
        }
        self.push_task_history(vec![(None, Some(copy.clone()))]);
        let copy_id = copy.id;
        self.tasks.push(copy);
        self.select_task(copy_id, window, cx);
        self.status_message = "タスクを複製しました".to_owned();
        cx.notify();
    }

    pub(super) fn swap_task_order(
        &mut self,
        dragged: TaskId,
        target: TaskId,
        cx: &mut Context<Self>,
    ) {
        if self.view_kind == ViewKind::List {
            let tasks = self.visible_tasks(cx);
            let Some(left) = tasks.iter().find(|task| task.id == dragged) else {
                return;
            };
            let Some(right) = tasks.iter().find(|task| task.id == target) else {
                return;
            };
            if !self.can_reorder_list_pair(left, right) {
                return;
            }
        }
        self.persist_task_order_swap(dragged, target, "ドラッグで表示順を変更しました", cx);
    }

    pub(super) fn persist_task_order_swap(
        &mut self,
        dragged: TaskId,
        target: TaskId,
        message: &str,
        cx: &mut Context<Self>,
    ) {
        if dragged == target {
            return;
        }
        let Some(left) = self.tasks.iter().position(|task| task.id == dragged) else {
            return;
        };
        let Some(right) = self.tasks.iter().position(|task| task.id == target) else {
            return;
        };
        let before_left = self.tasks[left].clone();
        let before_right = self.tasks[right].clone();
        let order = self.tasks[left].sort_order;
        self.tasks[left].sort_order = self.tasks[right].sort_order;
        self.tasks[right].sort_order = order;
        let changed = vec![self.tasks[left].clone(), self.tasks[right].clone()];
        if let Err(error) = self.worker.save_tasks(changed) {
            self.tasks[left] = before_left;
            self.tasks[right] = before_right;
            self.set_error(error);
        } else {
            self.push_task_history(vec![
                (Some(before_left), Some(self.tasks[left].clone())),
                (Some(before_right), Some(self.tasks[right].clone())),
            ]);
            self.status_message = message.to_owned();
        }
        cx.notify();
    }

    pub(super) fn toggle_task_selection(
        &mut self,
        id: TaskId,
        selected: bool,
        cx: &mut Context<Self>,
    ) {
        if selected {
            self.selected_tasks.insert(id);
        } else {
            self.selected_tasks.remove(&id);
        }
        cx.notify();
    }

    fn bulk_update(&mut self, cx: &mut Context<Self>, update: impl Fn(&mut Task, OffsetDateTime)) {
        let now = OffsetDateTime::now_utc();
        let mut changed = Vec::new();
        let mut history = Vec::new();
        for task in &mut self.tasks {
            if self.selected_tasks.contains(&task.id) {
                let before = task.clone();
                update(task, now);
                if before == *task {
                    continue;
                }
                history.push((Some(before), Some(task.clone())));
                changed.push(task.clone());
            }
        }
        if changed.is_empty() {
            return;
        }
        if let Err(error) = self.worker.save_tasks(changed) {
            for (before, _) in &history {
                if let Some(before) = before
                    && let Some(task) = self.tasks.iter_mut().find(|task| task.id == before.id)
                {
                    *task = before.clone();
                }
            }
            self.set_error(error);
        } else {
            self.push_task_history(history);
            self.status_message = format!("{}件を一括更新しました", self.selected_tasks.len());
        }
        cx.notify();
    }

    pub(super) fn bulk_status(&mut self, status: TaskStatus, cx: &mut Context<Self>) {
        self.bulk_update(cx, |task, now| task.set_status(status, now));
    }

    fn bulk_priority(&mut self, priority: Priority, cx: &mut Context<Self>) {
        self.bulk_update(cx, |task, now| {
            task.priority = priority;
            task.touch(now);
        });
    }

    fn bulk_due(&mut self, cx: &mut Context<Self>) {
        let value = self.bulk_due_input.read(cx).value().to_string();
        match parse_due(&value) {
            Ok(due) => {
                self.bulk_update(cx, |task, now| {
                    task.due = due.clone();
                    task.touch(now);
                });
                self.error_message = None;
            }
            Err(message) => {
                self.error_message = Some(message);
                cx.notify();
            }
        }
    }

    fn bulk_trash(&mut self, cx: &mut Context<Self>) {
        self.bulk_update(cx, |task, now| task.move_to_trash(now));
        self.selected_tasks.clear();
    }

    pub(super) fn handle_task_click(
        &mut self,
        id: TaskId,
        shift: bool,
        secondary: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if (shift || secondary) && !self.flush_pending_edits(cx) {
            return;
        }
        if shift {
            let tasks = self.visible_tasks(cx);
            let anchor = self.selection_anchor.or(self.selected_task).unwrap_or(id);
            let Some(anchor_index) = tasks.iter().position(|task| task.id == anchor) else {
                self.select_task(id, window, cx);
                return;
            };
            let Some(clicked_index) = tasks.iter().position(|task| task.id == id) else {
                return;
            };
            let (start, end) = if anchor_index <= clicked_index {
                (anchor_index, clicked_index)
            } else {
                (clicked_index, anchor_index)
            };
            self.selection_mode = true;
            if !secondary {
                self.selected_tasks.clear();
            }
            self.selected_tasks
                .extend(tasks[start..=end].iter().map(|task| task.id));
            self.selected_task = Some(id);
            self.sync_detail_inputs(window, cx);
            cx.notify();
        } else if secondary {
            self.selection_mode = true;
            if !self.selected_tasks.insert(id) {
                self.selected_tasks.remove(&id);
            }
            self.selected_task = Some(id);
            self.selection_anchor = Some(id);
            self.sync_detail_inputs(window, cx);
            cx.notify();
        } else {
            self.select_task(id, window, cx);
        }
    }

    pub(super) fn set_task_status(
        &mut self,
        id: TaskId,
        status: TaskStatus,
        cx: &mut Context<Self>,
    ) {
        self.update_task(id, cx, |task, now| task.set_status(status, now));
    }

    pub(super) fn set_task_priority(
        &mut self,
        id: TaskId,
        priority: Priority,
        cx: &mut Context<Self>,
    ) {
        self.update_task(id, cx, |task, now| {
            task.priority = priority;
            task.touch(now);
        });
    }

    pub(super) fn set_task_progress(&mut self, id: TaskId, progress: u8, cx: &mut Context<Self>) {
        self.update_task(id, cx, |task, now| {
            let _ = task.set_progress(progress);
            task.touch(now);
        });
    }

    pub(super) fn move_to_trash(&mut self, id: TaskId, cx: &mut Context<Self>) {
        let now = OffsetDateTime::now_utc();
        let mut history = None;
        if let Some(task) = self.tasks.iter_mut().find(|task| task.id == id) {
            let before = task.clone();
            task.move_to_trash(now);
            history = Some((Some(before), Some(task.clone())));
        }
        match self.worker.move_task_to_trash(id, now) {
            Err(error) => {
                if let Some((Some(before), _)) = &history
                    && let Some(task) = self.tasks.iter_mut().find(|task| task.id == id)
                {
                    *task = before.clone();
                }
                self.set_error(error);
            }
            Ok(()) => {
                if self.selected_task == Some(id) {
                    self.selected_task = None;
                }
                if let Some(change) = history {
                    self.push_task_history(vec![change]);
                }
                self.status_message = "ゴミ箱へ移動しました".to_owned();
            }
        }
        cx.notify();
    }

    pub(super) fn restore_task(&mut self, id: TaskId, cx: &mut Context<Self>) {
        let now = OffsetDateTime::now_utc();
        let mut history = None;
        if let Some(task) = self.tasks.iter_mut().find(|task| task.id == id) {
            let before = task.clone();
            task.restore(now);
            history = Some((Some(before), Some(task.clone())));
        }
        match self.worker.restore_task(id, now) {
            Err(error) => {
                if let Some((Some(before), _)) = &history
                    && let Some(task) = self.tasks.iter_mut().find(|task| task.id == id)
                {
                    *task = before.clone();
                }
                self.set_error(error);
            }
            Ok(()) => {
                if let Some(change) = history {
                    self.push_task_history(vec![change]);
                }
                self.status_message = "タスクを復元しました".to_owned();
            }
        }
        cx.notify();
    }

    pub(super) fn update_selected_task(
        &mut self,
        cx: &mut Context<Self>,
        update: impl FnOnce(&mut Task, OffsetDateTime),
    ) {
        if let Some(id) = self.selected_task {
            self.update_task(id, cx, update);
        }
    }

    fn update_task(
        &mut self,
        id: TaskId,
        cx: &mut Context<Self>,
        update: impl FnOnce(&mut Task, OffsetDateTime),
    ) {
        let now = OffsetDateTime::now_utc();
        let Some(task) = self.tasks.iter_mut().find(|task| task.id == id) else {
            return;
        };
        let before = task.clone();
        update(task, now);
        if before == *task {
            return;
        }
        let task = task.clone();
        if let Err(error) = self.worker.save_task(task) {
            if let Some(task) = self.tasks.iter_mut().find(|task| task.id == id) {
                *task = before;
            }
            self.set_error(error);
        } else {
            let after = self
                .tasks
                .iter()
                .find(|task| task.id == id)
                .cloned()
                .expect("updated task must exist");
            self.push_task_history(vec![(Some(before), Some(after))]);
            self.status_message = "保存中…".to_owned();
        }
        cx.notify();
    }

    pub(super) fn toggle_selected_done(&mut self, cx: &mut Context<Self>) {
        let Some(task) = self.selected_task().cloned() else {
            return;
        };
        self.set_task_status(
            task.id,
            if task.status == TaskStatus::Done {
                TaskStatus::Todo
            } else {
                TaskStatus::Done
            },
            cx,
        );
    }

    pub(super) fn render_bulk_bar(&self, cx: &mut Context<Self>) -> AnyElement {
        let count = self.selected_tasks.len();
        div()
            .flex()
            .items_center()
            .flex_wrap()
            .gap_2()
            .px_4()
            .pb_3()
            .child(format!("{count}件を選択"))
            .child(self.small_action_button(
                "bulk-all",
                "表示中を全選択",
                cx,
                |this, _, cx| {
                    this.selected_tasks = this
                        .visible_tasks(cx)
                        .into_iter()
                        .map(|task| task.id)
                        .collect();
                    cx.notify();
                },
            ))
            .children(TaskStatus::ALL.into_iter().map(|status| {
                let entity = cx.entity();
                Button::new(SharedString::from(format!(
                    "bulk-status-{}",
                    status.as_str()
                )))
                .small()
                .label(format!("状態 {}", status.label()))
                .on_click(move |_, _, cx| {
                    entity.update(cx, |this, cx| this.bulk_status(status, cx));
                })
            }))
            .children(Priority::ALL.into_iter().map(|priority| {
                let entity = cx.entity();
                Button::new(SharedString::from(format!(
                    "bulk-priority-{}",
                    priority.as_str()
                )))
                .small()
                .label(format!("優先度 {}", priority.label()))
                .on_click(move |_, _, cx| {
                    entity.update(cx, |this, cx| this.bulk_priority(priority, cx));
                })
            }))
            .child(Input::new(&self.bulk_due_input).small().w(px(260.0)))
            .child(
                self.small_action_button("bulk-due", "納期を設定", cx, |this, _, cx| {
                    this.bulk_due(cx);
                }),
            )
            .child(
                self.small_action_button("bulk-due-clear", "納期なし", cx, |this, _, cx| {
                    this.bulk_update(cx, |task, now| {
                        task.due = Due::None;
                        task.touch(now);
                    });
                }),
            )
            .child({
                let entity = cx.entity();
                Button::new("bulk-trash")
                    .small()
                    .danger()
                    .label("まとめて削除")
                    .on_click(move |_, _, cx| {
                        entity.update(cx, |this, cx| this.bulk_trash(cx));
                    })
            })
            .into_any_element()
    }
}
