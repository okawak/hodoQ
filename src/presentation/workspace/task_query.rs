use crate::domain::{GroupBy, Priority, SavedBaseView, SavedView, Task};

/// Keep legacy classification views on disk, without applying invisible filters in the UI.
pub(super) fn saved_view_is_available(view: &SavedView) -> bool {
    let filter = &view.filter;
    !matches!(
        filter.base_view,
        Some(SavedBaseView::Project(_) | SavedBaseView::Tag(_))
    ) && filter.project_ids.is_empty()
        && !filter.unassigned_project
        && filter.tag_ids.is_empty()
        && view.group_by != Some(GroupBy::Project)
}

pub(super) fn prioritize_list_tasks(tasks: &mut [Task]) {
    tasks.sort_by_key(|task| task.priority != Priority::High);
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::OffsetDateTime;

    #[test]
    fn list_moves_high_priority_first_and_preserves_the_other_order() {
        let now = OffsetDateTime::now_utc();
        let mut tasks = vec![
            task("low-first", Priority::Low, now),
            task("high-first", Priority::High, now),
            task("medium", Priority::Medium, now),
            task("low-second", Priority::Low, now),
            task("high-second", Priority::High, now),
            task("none", Priority::None, now),
        ];

        prioritize_list_tasks(&mut tasks);

        assert_eq!(
            tasks.into_iter().map(|task| task.title).collect::<Vec<_>>(),
            [
                "high-first",
                "high-second",
                "low-first",
                "medium",
                "low-second",
                "none"
            ]
        );
    }

    fn task(title: &str, priority: Priority, now: OffsetDateTime) -> Task {
        let mut task = Task::new(title, now).unwrap();
        task.priority = priority;
        task
    }
}
