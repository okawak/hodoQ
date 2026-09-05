use gpui::{
    AnyElement, AppContext as _, Context, FontWeight, InteractiveElement as _, IntoElement,
    ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _, div,
    prelude::FluentBuilder as _, px,
};
use gpui_component::{
    Disableable as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    menu::{ContextMenuExt as _, PopupMenuItem},
    progress::Progress,
};
use time::{OffsetDateTime, UtcOffset};

use crate::domain::{Due, GroupBy, Priority, SortField, Task, TaskId, TaskStatus};

use super::theme;

use super::due::{due_date, format_due_display};
use super::task_query::prioritize_list_tasks;
use super::{SmartView, TaskDrag, Workspace, priority_color};
use gpui::uniform_list;
#[cfg(test)]
mod tests;

#[derive(Clone)]
enum VirtualListItem {
    Group(String),
    Task(usize),
}

impl Workspace {
    pub(super) fn can_reorder_list_pair(&self, left: &Task, right: &Task) -> bool {
        // Manual ranks cannot override the selected sort, grouping, or pinned priority.
        self.sort
            .first()
            .is_some_and(|sort| sort.field == SortField::Manual)
            && left.deleted_at.is_none()
            && right.deleted_at.is_none()
            && left.sort_order != right.sort_order
            && (left.priority == Priority::High) == (right.priority == Priority::High)
            && self.group_by.is_none_or(|group| {
                self.task_group_label(left, group) == self.task_group_label(right, group)
            })
    }

    fn list_move_target<'a>(
        &self,
        tasks: &'a [Task],
        index: usize,
        direction: i32,
    ) -> Option<&'a Task> {
        let target = match direction {
            -1 => index.checked_sub(1)?,
            1 => index.checked_add(1)?,
            _ => return None,
        };
        let target = tasks.get(target)?;
        self.can_reorder_list_pair(tasks.get(index)?, target)
            .then_some(target)
    }

    pub(super) fn move_task_order(&mut self, id: TaskId, direction: i32, cx: &mut Context<Self>) {
        let tasks = self.visible_tasks(cx);
        let Some(index) = tasks.iter().position(|task| task.id == id) else {
            return;
        };
        let Some(target) = self.list_move_target(&tasks, index, direction) else {
            return;
        };
        self.persist_task_order_swap(id, target.id, "表示順を変更しました", cx);
    }

    fn render_list_task(&self, tasks: &[Task], index: usize, cx: &mut Context<Self>) -> AnyElement {
        self.render_task_row(
            tasks[index].clone(),
            self.list_move_target(tasks, index, -1).is_some(),
            self.list_move_target(tasks, index, 1).is_some(),
            cx,
        )
    }

    /// Order the shared sequence before rendering or handling list interactions.
    pub(super) fn order_list_tasks(&self, tasks: &mut [Task]) {
        prioritize_list_tasks(tasks);
        if let Some(group) = self.group_by {
            tasks.sort_by_key(|task| self.task_group_label(task, group));
        }
    }

    pub(super) fn render_list(&self, tasks: Vec<Task>, cx: &mut Context<Self>) -> AnyElement {
        let title = self.active_view_label();
        let count = tasks.len();
        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .px_4()
            .pt_4()
            .pb_2()
            .child(
                div()
                    .text_size(px(20.0))
                    .font_weight(FontWeight::BOLD)
                    .child(title),
            )
            .child(format!("{count} 件"));

        if self.group_by.is_none() {
            let tasks = std::sync::Arc::new(tasks);
            return div()
                .flex()
                .flex_col()
                .flex_1()
                .h_full()
                .min_h_0()
                .child(header)
                .when(count == 0, |container| {
                    container.child(
                        div()
                            .p_8()
                            .text_color(theme::MUTED)
                            .child("該当するタスクはありません"),
                    )
                })
                .when(count > 0, |container| {
                    container.child(
                        uniform_list(
                            "task-list",
                            count,
                            cx.processor(
                                move |this, range: std::ops::Range<usize>, _window, cx| {
                                    range
                                        .map(|index| this.render_list_task(&tasks, index, cx))
                                        .collect::<Vec<_>>()
                                },
                            ),
                        )
                        .flex_1()
                        .px_4()
                        .pb_4(),
                    )
                })
                .into_any_element();
        }

        let mut items = Vec::new();
        let mut current_group = None::<String>;
        for (index, task) in tasks.iter().enumerate() {
            if let Some(group) = self.group_by {
                let label = self.task_group_label(task, group);
                if current_group.as_deref() != Some(label.as_str()) {
                    current_group = Some(label.clone());
                    items.push(VirtualListItem::Group(label));
                }
            }
            items.push(VirtualListItem::Task(index));
        }
        let item_count = items.len();
        let items = std::sync::Arc::new(items);
        let tasks = std::sync::Arc::new(tasks);
        div()
            .flex()
            .flex_col()
            .flex_1()
            .h_full()
            .min_h_0()
            .child(header)
            .when(count == 0, |container| {
                container.child(
                    div()
                        .p_8()
                        .text_color(theme::MUTED)
                        .child("該当するタスクはありません"),
                )
            })
            .when(count > 0, |container| {
                container.child(
                    uniform_list(
                        "grouped-task-list",
                        item_count,
                        cx.processor(move |this, range: std::ops::Range<usize>, _window, cx| {
                            range
                                .map(|index| match items[index].clone() {
                                    VirtualListItem::Group(label) => div()
                                        .mt_3()
                                        .text_color(theme::MUTED)
                                        .font_weight(FontWeight::BOLD)
                                        .child(label)
                                        .into_any_element(),
                                    VirtualListItem::Task(index) => {
                                        this.render_list_task(&tasks, index, cx)
                                    }
                                })
                                .collect::<Vec<_>>()
                        }),
                    )
                    .flex_1()
                    .px_4()
                    .pb_4(),
                )
            })
            .into_any_element()
    }

    fn task_group_label(&self, task: &Task, group: GroupBy) -> String {
        match group {
            GroupBy::Status => task.status.label().to_owned(),
            GroupBy::Project => task
                .project_id
                .and_then(|id| self.projects.iter().find(|project| project.id == id))
                .map(|project| project.name.clone())
                .unwrap_or_else(|| "プロジェクトなし".to_owned()),
            GroupBy::Priority => format!("優先度 {}", task.priority.label()),
            GroupBy::Due => match &task.due {
                Due::None => "納期未定".to_owned(),
                Due::Date(date) => date.to_string(),
                Due::DateTime(date_time) => date_time.date().to_string(),
            },
        }
    }
}

impl Workspace {
    fn render_task_row(
        &self,
        task: Task,
        can_move_up: bool,
        can_move_down: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let task_id = task.id;
        let selected =
            self.selected_task == Some(task_id) || self.selected_tasks.contains(&task_id);
        let selection_mode = self.selection_mode;
        let multi_selected = self.selected_tasks.contains(&task_id);
        let checkbox_entity = cx.entity();
        let row_entity = cx.entity();
        let context_entity = cx.entity();
        let drop_entity = cx.entity();
        let drag_task = TaskDrag {
            id: task_id,
            title: task.title.clone(),
        };
        let due = format_due_display(&task.due);
        let row_debug_selector = if due.is_empty() {
            "task-row-undated"
        } else {
            "task-row-dated"
        };
        let priority_color = priority_color(task.priority);
        let now = OffsetDateTime::now_utc();
        let offset = UtcOffset::current_local_offset().unwrap_or(UtcOffset::UTC);
        let today = now.to_offset(offset).date();
        let due_color = if task.status != TaskStatus::Done && task.due.is_overdue(now, today) {
            theme::DANGER
        } else if due_date(&task.due)
            .is_some_and(|date| date == today || date == today + time::Duration::days(1))
        {
            theme::WARNING
        } else {
            theme::MUTED
        };
        div()
            .id(SharedString::from(format!("task-{task_id}")))
            .debug_selector(move || row_debug_selector.to_owned())
            .flex()
            .flex_col()
            .w_full()
            .min_w_0()
            .gap_3()
            .p_3()
            .rounded_lg()
            .border_1()
            .border_color(if selected {
                theme::ACCENT
            } else {
                theme::BORDER
            })
            .bg(if selected {
                theme::SURFACE_HOVER
            } else {
                theme::SURFACE
            })
            .cursor_move()
            .hover(|style| style.bg(theme::SURFACE_HOVER))
            .on_drag(drag_task, |task, _, _, cx| {
                let task = task.clone();
                cx.new(|_| task)
            })
            .on_drop(move |dragged: &TaskDrag, _, cx| {
                drop_entity.update(cx, |this, cx| {
                    this.swap_task_order(dragged.id, task_id, cx);
                });
            })
            .on_click(
                cx.listener(move |this, event: &gpui::ClickEvent, window, cx| {
                    let modifiers = event.modifiers();
                    this.handle_task_click(
                        task_id,
                        modifiers.shift,
                        modifiers.secondary(),
                        window,
                        cx,
                    );
                }),
            )
            .child(
                div()
                    .flex()
                    .w_full()
                    .min_w_0()
                    .items_center()
                    .gap_3()
                    .child(
                        Checkbox::new(SharedString::from(format!("complete-{task_id}")))
                            .checked(if selection_mode {
                                multi_selected
                            } else {
                                task.status == TaskStatus::Done
                            })
                            .on_click(move |checked, _, cx| {
                                checkbox_entity.update(cx, |this, cx| {
                                    if selection_mode {
                                        this.toggle_task_selection(task_id, *checked, cx);
                                    } else {
                                        this.set_task_status(
                                            task_id,
                                            if *checked {
                                                TaskStatus::Done
                                            } else {
                                                TaskStatus::Todo
                                            },
                                            cx,
                                        );
                                    }
                                });
                            }),
                    )
                    .child(
                        div()
                            .debug_selector(move || format!("task-info-{task_id}"))
                            .flex()
                            .flex_col()
                            .gap_1()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .text_ellipsis()
                                    .text_color(if task.status == TaskStatus::Done {
                                        theme::MUTED
                                    } else {
                                        theme::TEXT
                                    })
                                    .font_weight(FontWeight::MEDIUM)
                                    .when(task.status == TaskStatus::Done, |title| {
                                        title.line_through()
                                    })
                                    .child(task.title),
                            )
                            .child(
                                div()
                                    .flex()
                                    .min_w_0()
                                    .items_center()
                                    .gap_3()
                                    .text_size(px(12.0))
                                    .text_color(theme::MUTED)
                                    .child(div().flex_1().min_w_0().text_ellipsis().child(
                                        if task.status == TaskStatus::Blocked {
                                            "⏸ 保留"
                                        } else {
                                            task.status.label()
                                        },
                                    ))
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .text_ellipsis()
                                            .text_color(priority_color)
                                            .child(format!("優先度 {}", task.priority.label())),
                                    ),
                            )
                            .child(
                                div()
                                    .debug_selector(move || format!("task-due-{task_id}"))
                                    .w_full()
                                    .min_w_0()
                                    .max_w(px(176.0))
                                    .text_ellipsis()
                                    .text_size(px(12.0))
                                    .text_color(due_color)
                                    .child(if due.is_empty() {
                                        "納期なし".to_owned()
                                    } else {
                                        due
                                    }),
                            )
                            .child(Progress::new().value(f32::from(task.progress))),
                    ),
            )
            // Every row uses the same single-line metadata and wrapping action layout,
            // keeping uniform_list item heights independent of title and due contents.
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .w_full()
                    .min_w_0()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .w(px(44.0))
                            .flex_shrink_0()
                            .child(format!("{}%", task.progress)),
                    )
                    .child({
                        let entity = cx.entity();
                        Button::new(SharedString::from(format!("move-up-{task_id}")))
                            .ghost()
                            .small()
                            .label("↑")
                            .disabled(!can_move_up)
                            .on_click(move |_, _, cx| {
                                entity.update(cx, |this, cx| this.move_task_order(task_id, -1, cx));
                            })
                    })
                    .child({
                        let entity = cx.entity();
                        Button::new(SharedString::from(format!("move-down-{task_id}")))
                            .ghost()
                            .small()
                            .label("↓")
                            .disabled(!can_move_down)
                            .on_click(move |_, _, cx| {
                                entity.update(cx, |this, cx| this.move_task_order(task_id, 1, cx));
                            })
                    })
                    .child({
                        let entity = cx.entity();
                        Button::new(SharedString::from(format!("duplicate-{task_id}")))
                            .ghost()
                            .small()
                            .label("複製")
                            .on_click(move |_, window, cx| {
                                entity.update(cx, |this, cx| {
                                    this.duplicate_task(task_id, window, cx)
                                });
                            })
                    })
                    .when(
                        !matches!(self.active_view, SmartView::Trash | SmartView::Archived),
                        |row| {
                            let entity = cx.entity();
                            row.child(
                                Button::new(SharedString::from(format!("archive-{task_id}")))
                                    .ghost()
                                    .small()
                                    .label("保管")
                                    .on_click(move |_, _, cx| {
                                        entity.update(cx, |this, cx| {
                                            this.set_task_status(task_id, TaskStatus::Archived, cx);
                                        });
                                    }),
                            )
                        },
                    )
                    .when(self.active_view == SmartView::Archived, |row| {
                        let entity = cx.entity();
                        row.child(
                            Button::new(SharedString::from(format!("unarchive-{task_id}")))
                                .ghost()
                                .small()
                                .label("未着手へ")
                                .on_click(move |_, _, cx| {
                                    entity.update(cx, |this, cx| {
                                        this.set_task_status(task_id, TaskStatus::Todo, cx);
                                    });
                                }),
                        )
                    })
                    .child(
                        Button::new(SharedString::from(format!("delete-{task_id}")))
                            .debug_selector(move || format!("task-delete-{task_id}"))
                            .ghost()
                            .danger()
                            .small()
                            .label(if self.active_view == SmartView::Trash {
                                "復元"
                            } else {
                                "削除"
                            })
                            .on_click(move |_, _, cx| {
                                row_entity.update(cx, |this, cx| {
                                    if this.active_view == SmartView::Trash {
                                        this.restore_task(task_id, cx);
                                    } else {
                                        this.move_to_trash(task_id, cx);
                                    }
                                });
                            }),
                    ),
            )
            .context_menu(move |menu, _, _| {
                let edit_entity = context_entity.clone();
                let done_entity = context_entity.clone();
                let duplicate_entity = context_entity.clone();
                let archive_entity = context_entity.clone();
                let delete_entity = context_entity.clone();
                menu.item(
                    PopupMenuItem::new("編集を開く").on_click(move |_, window, cx| {
                        edit_entity.update(cx, |this, cx| this.select_task(task_id, window, cx));
                    }),
                )
                .item(
                    PopupMenuItem::new(if task.status == TaskStatus::Done {
                        "未完了へ戻す"
                    } else {
                        "完了にする"
                    })
                    .on_click(move |_, _, cx| {
                        done_entity.update(cx, |this, cx| {
                            this.set_task_status(
                                task_id,
                                if task.status == TaskStatus::Done {
                                    TaskStatus::Todo
                                } else {
                                    TaskStatus::Done
                                },
                                cx,
                            );
                        });
                    }),
                )
                .item(PopupMenuItem::new("複製").on_click(move |_, window, cx| {
                    duplicate_entity
                        .update(cx, |this, cx| this.duplicate_task(task_id, window, cx));
                }))
                .item(
                    PopupMenuItem::new(if task.status == TaskStatus::Archived {
                        "未着手へ戻す"
                    } else {
                        "アーカイブ"
                    })
                    .on_click(move |_, _, cx| {
                        archive_entity.update(cx, |this, cx| {
                            this.set_task_status(
                                task_id,
                                if task.status == TaskStatus::Archived {
                                    TaskStatus::Todo
                                } else {
                                    TaskStatus::Archived
                                },
                                cx,
                            );
                        });
                    }),
                )
                .separator()
                .item(
                    PopupMenuItem::new(if task.deleted_at.is_some() {
                        "ゴミ箱から復元"
                    } else {
                        "ゴミ箱へ移動"
                    })
                    .on_click(move |_, _, cx| {
                        delete_entity.update(cx, |this, cx| {
                            if task.deleted_at.is_some() {
                                this.restore_task(task_id, cx);
                            } else {
                                this.move_to_trash(task_id, cx);
                            }
                        });
                    }),
                )
            })
            .into_any_element()
    }
}
