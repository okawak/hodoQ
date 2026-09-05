use gpui::{
    AnyElement, Context, FontWeight, IntoElement, ParentElement as _, Styled as _, div,
    prelude::FluentBuilder as _, px, uniform_list,
};

use crate::domain::{Due, GroupBy, Priority, SortField, Task, TaskId};

use super::{Workspace, prioritize_list_tasks, theme};

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
