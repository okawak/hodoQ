use super::{Workspace, labeled_input, section_label, theme};
use crate::domain::{Priority, TaskId, TaskStatus};
use gpui::{
    AnyElement, Context, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _,
    SharedString, Styled as _, Window, div, px,
};
use gpui_component::{
    Selectable as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    input::Input,
    progress::Progress,
    scroll::ScrollableElement as _,
};

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
