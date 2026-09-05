use gpui::{
    AnyElement, Context, FontWeight, IntoElement, ParentElement as _, Styled as _, div,
    prelude::FluentBuilder as _, px, uniform_list,
};

use crate::domain::{Due, GroupBy, Task};

use super::{Workspace, prioritize_list_tasks, theme};

#[cfg(test)]
mod tests;

#[derive(Clone)]
enum VirtualListItem {
    Group(String),
    Task(Task),
}

impl Workspace {
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
                                        .map(|index| this.render_task_row(tasks[index].clone(), cx))
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
        for task in tasks {
            if let Some(group) = self.group_by {
                let label = self.task_group_label(&task, group);
                if current_group.as_deref() != Some(label.as_str()) {
                    current_group = Some(label.clone());
                    items.push(VirtualListItem::Group(label));
                }
            }
            items.push(VirtualListItem::Task(task));
        }
        let item_count = items.len();
        let items = std::sync::Arc::new(items);
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
                                    VirtualListItem::Task(task) => this.render_task_row(task, cx),
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
