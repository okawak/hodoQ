//! Command palette execution and keyboard selection.
use gpui::{
    AnyElement, Context, Focusable as _, IntoElement, ParentElement as _, SharedString,
    Styled as _, Window, div, px,
};
use gpui_component::{Sizable as _, button::Button, input::Input};

use crate::domain::{Priority, TaskStatus};

use super::theme;

use super::{PaletteCommand, SmartView, Workspace};

impl Workspace {
    pub(super) fn render_command_palette(&self, cx: &mut Context<Self>) -> AnyElement {
        let query = self.command_input.read(cx).value().to_lowercase();
        let commands = [
            (PaletteCommand::NewTask, "新しいタスク", "新規 追加 task"),
            (PaletteCommand::Search, "タスクを検索", "検索 find search"),
            (PaletteCommand::Today, "今日へ移動", "今日 today view"),
            (PaletteCommand::All, "すべてへ移動", "全部 all view"),
            (
                PaletteCommand::StatusTodo,
                "選択タスクを未着手へ",
                "状態 todo 未着手",
            ),
            (
                PaletteCommand::StatusDoing,
                "選択タスクを進行中へ",
                "状態 doing 進行",
            ),
            (
                PaletteCommand::StatusBlocked,
                "選択タスクを保留へ",
                "状態 blocked 保留",
            ),
            (
                PaletteCommand::StatusDone,
                "選択タスクを完了へ",
                "状態 done 完了",
            ),
            (
                PaletteCommand::StatusArchived,
                "選択タスクをアーカイブへ",
                "状態 archived アーカイブ",
            ),
            (
                PaletteCommand::PriorityNone,
                "選択タスクの優先度をなしへ",
                "優先度 none なし",
            ),
            (
                PaletteCommand::PriorityLow,
                "選択タスクを低優先度へ",
                "優先度 low 低",
            ),
            (
                PaletteCommand::PriorityMedium,
                "選択タスクを中優先度へ",
                "優先度 medium 中",
            ),
            (
                PaletteCommand::PriorityHigh,
                "選択タスクを高優先度へ",
                "優先度 high 高",
            ),
            (
                PaletteCommand::Backup,
                "手動バックアップを作成",
                "backup 保存 バックアップ",
            ),
        ];
        div()
            .flex()
            .flex_col()
            .gap_2()
            .px_4()
            .pb_3()
            .border_t_1()
            .border_color(theme::BORDER)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .pt_3()
                    .child(Input::new(&self.command_input).cleanable(true).w(px(420.0)))
                    .child(self.small_action_button(
                        "close-command-palette",
                        "閉じる",
                        cx,
                        |this, _, cx| {
                            this.show_command_palette = false;
                            cx.notify();
                        },
                    )),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .children(
                        commands
                            .into_iter()
                            .filter_map(|(command, label, keywords)| {
                                let haystack = format!("{label} {keywords}").to_lowercase();
                                if !query.is_empty() && !haystack.contains(query.trim()) {
                                    return None;
                                }
                                let entity = cx.entity();
                                Some(
                                    Button::new(SharedString::from(format!("palette-{command:?}")))
                                        .small()
                                        .label(label)
                                        .on_click(move |_, window, cx| {
                                            entity.update(cx, |this, cx| {
                                                this.execute_palette_command(command, window, cx);
                                            });
                                        }),
                                )
                            }),
                    ),
            )
            .into_any_element()
    }

    fn execute_palette_command(
        &mut self,
        command: PaletteCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match command {
            PaletteCommand::NewTask => self.open_new_task_form(window, cx),
            PaletteCommand::Search => self.search_input.read(cx).focus_handle(cx).focus(window),
            PaletteCommand::Today => self.active_view = SmartView::Today,
            PaletteCommand::All => self.active_view = SmartView::All,
            PaletteCommand::StatusTodo => self.set_selected_task_status(TaskStatus::Todo, cx),
            PaletteCommand::StatusDoing => self.set_selected_task_status(TaskStatus::Doing, cx),
            PaletteCommand::StatusBlocked => self.set_selected_task_status(TaskStatus::Blocked, cx),
            PaletteCommand::StatusDone => self.set_selected_task_status(TaskStatus::Done, cx),
            PaletteCommand::StatusArchived => {
                self.set_selected_task_status(TaskStatus::Archived, cx)
            }
            PaletteCommand::PriorityNone => self.set_selected_task_priority(Priority::None, cx),
            PaletteCommand::PriorityLow => self.set_selected_task_priority(Priority::Low, cx),
            PaletteCommand::PriorityMedium => self.set_selected_task_priority(Priority::Medium, cx),
            PaletteCommand::PriorityHigh => self.set_selected_task_priority(Priority::High, cx),
            PaletteCommand::Backup => self.create_manual_backup(cx),
        }
        self.show_command_palette = false;
        self.command_input
            .update(cx, |state, cx| state.set_value("", window, cx));
        cx.notify();
    }

    fn set_selected_task_status(&mut self, status: TaskStatus, cx: &mut Context<Self>) {
        if let Some(id) = self.selected_task {
            self.set_task_status(id, status, cx);
        }
    }

    fn set_selected_task_priority(&mut self, priority: Priority, cx: &mut Context<Self>) {
        if let Some(id) = self.selected_task {
            self.set_task_priority(id, priority, cx);
        }
    }

    pub(super) fn move_selection(
        &mut self,
        direction: i32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let tasks = self.visible_tasks(cx);
        if tasks.is_empty() {
            return;
        }
        let current = self
            .selected_task
            .and_then(|id| tasks.iter().position(|task| task.id == id));
        let index = match (current, direction.is_negative()) {
            (Some(index), true) => index.saturating_sub(1),
            (Some(index), false) => (index + 1).min(tasks.len() - 1),
            (None, true) => tasks.len() - 1,
            (None, false) => 0,
        };
        self.select_task(tasks[index].id, window, cx);
    }
}
