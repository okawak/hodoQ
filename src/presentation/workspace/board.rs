//! Status columns and draggable board cards.
use gpui::{
    AnyElement, AppContext as _, Context, FontWeight, InteractiveElement as _, IntoElement,
    ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _, div,
    prelude::FluentBuilder as _, px,
};
use gpui_component::{Sizable as _, button::Button, scroll::ScrollableElement as _};

use crate::domain::{Task, TaskStatus};

use super::theme;

use super::due::format_due_display;
use super::{TaskDrag, Workspace};

impl Workspace {
    pub(super) fn render_board(&self, tasks: Vec<Task>, cx: &mut Context<Self>) -> AnyElement {
        let columns = [
            TaskStatus::Todo,
            TaskStatus::Doing,
            TaskStatus::Blocked,
            TaskStatus::Done,
        ];
        div()
            .flex()
            .flex_1()
            .h_full()
            .gap_3()
            .p_4()
            .overflow_x_scrollbar()
            .children(columns.into_iter().map(|status| {
                let drop_entity = cx.entity();
                let column_tasks = tasks
                    .iter()
                    .filter(|task| task.status == status)
                    .cloned()
                    .collect::<Vec<_>>();
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .w(px(280.0))
                    .min_w(px(280.0))
                    .p_3()
                    .rounded_lg()
                    .bg(theme::SURFACE)
                    .border_1()
                    .border_color(theme::BORDER)
                    .on_drop(move |dragged: &TaskDrag, _, cx| {
                        drop_entity.update(cx, |this, cx| {
                            this.set_task_status(dragged.id, status, cx);
                        });
                    })
                    .child(div().font_weight(FontWeight::BOLD).child(format!(
                        "{}  {}",
                        status.label(),
                        column_tasks.len()
                    )))
                    .children(
                        column_tasks
                            .into_iter()
                            .map(|task| self.render_board_card(task, cx)),
                    )
            }))
            .into_any_element()
    }

    fn render_board_card(&self, task: Task, cx: &mut Context<Self>) -> AnyElement {
        let id = task.id;
        let drop_entity = cx.entity();
        let drag_task = TaskDrag {
            id,
            title: task.title.clone(),
        };
        let states = [
            TaskStatus::Todo,
            TaskStatus::Doing,
            TaskStatus::Blocked,
            TaskStatus::Done,
        ];
        let position = states
            .iter()
            .position(|status| *status == task.status)
            .unwrap_or(0);
        let previous = position.checked_sub(1).map(|index| states[index]);
        let next = states.get(position + 1).copied();
        div()
            .id(SharedString::from(format!("board-{id}")))
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .rounded_md()
            .bg(theme::BACKGROUND)
            .border_1()
            .border_color(theme::BORDER)
            .cursor_move()
            .hover(|style| style.border_color(theme::ACCENT))
            .on_drag(drag_task, |task, _, _, cx| {
                let task = task.clone();
                cx.new(|_| task)
            })
            .on_drop(move |dragged: &TaskDrag, _, cx| {
                drop_entity.update(cx, |this, cx| {
                    this.swap_task_order(dragged.id, id, cx);
                });
            })
            .on_click(cx.listener(move |this, _, window, cx| {
                this.select_task(id, window, cx);
            }))
            .child(task.title)
            .child(
                div()
                    .text_size(px(12.0))
                    .text_color(theme::MUTED)
                    .child(format!(
                        "{} · {}% · {}",
                        task.priority.label(),
                        task.progress,
                        format_due_display(&task.due)
                    )),
            )
            .child(
                div()
                    .flex()
                    .justify_between()
                    .when_some(previous, |row, status| {
                        let entity = cx.entity();
                        row.child(
                            Button::new(SharedString::from(format!("board-prev-{id}")))
                                .small()
                                .label(format!("← {}", status.label()))
                                .on_click(move |_, _, cx| {
                                    entity.update(cx, |this, cx| {
                                        this.set_task_status(id, status, cx);
                                    });
                                }),
                        )
                    })
                    .when_some(next, |row, status| {
                        let entity = cx.entity();
                        row.child(
                            Button::new(SharedString::from(format!("board-next-{id}")))
                                .small()
                                .label(format!("{} →", status.label()))
                                .on_click(move |_, _, cx| {
                                    entity.update(cx, |this, cx| {
                                        this.set_task_status(id, status, cx);
                                    });
                                }),
                        )
                    }),
            )
            .into_any_element()
    }
}
