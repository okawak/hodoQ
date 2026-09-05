use std::cmp::Ordering;

use time::{OffsetDateTime, UtcOffset};

use crate::domain::{
    Due, DueScope, GroupBy, Priority, SavedBaseView, SavedView, SortDirection, SortField, SortSpec,
    Task, TaskStatus,
};

use super::{due_date, due_is_today, due_is_upcoming, status_filter_matches};

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

pub(super) fn task_matches_saved_view(task: &Task, view: &SavedView) -> bool {
    let filter = &view.filter;
    let base_view = filter.base_view;
    let visibility_matches = match base_view {
        Some(SavedBaseView::Trash) => task.deleted_at.is_some(),
        _ if filter.only_deleted => task.deleted_at.is_some(),
        _ => task.deleted_at.is_none(),
    };
    if !visibility_matches
        || (task.status == TaskStatus::Archived
            && !filter.only_deleted
            && base_view != Some(SavedBaseView::Archived)
            && base_view != Some(SavedBaseView::Trash)
            && !filter.include_archived)
    {
        return false;
    }
    let now = OffsetDateTime::now_utc();
    let offset = UtcOffset::current_local_offset().unwrap_or(UtcOffset::UTC);
    let today = now.to_offset(offset).date();
    let smart_view_matches = match base_view {
        None | Some(SavedBaseView::Trash) => true,
        Some(SavedBaseView::Inbox) => task.status == TaskStatus::Todo,
        Some(SavedBaseView::Today) => due_is_today(&task.due, today, offset),
        Some(SavedBaseView::Upcoming) => due_is_upcoming(&task.due, today, offset),
        Some(SavedBaseView::Overdue) => {
            task.status != TaskStatus::Done && task.due.is_overdue(now, today)
        }
        Some(SavedBaseView::Undated) => matches!(task.due, Due::None),
        Some(SavedBaseView::Doing) => task.status == TaskStatus::Doing,
        Some(SavedBaseView::Blocked) => task.status == TaskStatus::Blocked,
        Some(SavedBaseView::Done) => task.status == TaskStatus::Done,
        Some(SavedBaseView::Archived) => task.status == TaskStatus::Archived,
        Some(SavedBaseView::Project(id)) => task.project_id == Some(id),
        Some(SavedBaseView::Tag(id)) => task.tag_ids.contains(&id),
    };
    if !smart_view_matches {
        return false;
    }
    let query = filter.query.trim().to_lowercase();
    let base_matches = (query.is_empty()
        || task.title.to_lowercase().contains(&query)
        || task.memo.to_lowercase().contains(&query))
        && status_filter_matches(&filter.statuses, task.status)
        && (filter.priorities.is_empty() || filter.priorities.contains(&task.priority))
        && ((filter.project_ids.is_empty() && !filter.unassigned_project)
            || task.project_id.map_or(filter.unassigned_project, |id| {
                filter.project_ids.contains(&id)
            }))
        && (filter.tag_ids.is_empty()
            || if filter.match_all_tags {
                filter.tag_ids.iter().all(|id| task.tag_ids.contains(id))
            } else {
                filter.tag_ids.iter().any(|id| task.tag_ids.contains(id))
            });
    if !base_matches {
        return false;
    }
    let date = due_date(&task.due);
    let scope_matches = match filter.due_scope {
        DueScope::Any => true,
        DueScope::Undated => date.is_none(),
        DueScope::Today => date == Some(today),
        DueScope::Upcoming => {
            date.is_some_and(|date| date >= today && date <= today + time::Duration::days(7))
        }
        DueScope::Overdue => task.status != TaskStatus::Done && task.due.is_overdue(now, today),
    };
    scope_matches
        && filter
            .due_from
            .is_none_or(|from| date.is_some_and(|date| date >= from.to_offset(offset).date()))
        && filter
            .due_to
            .is_none_or(|to| date.is_some_and(|date| date <= to.to_offset(offset).date()))
}

pub(super) fn compare_tasks(left: &Task, right: &Task, sort: &[SortSpec]) -> Ordering {
    for sort in sort {
        let ordering = compare_task_field(left, right, sort.field);
        let ordering = match sort.direction {
            SortDirection::Ascending => ordering,
            SortDirection::Descending => ordering.reverse(),
        };
        if ordering != Ordering::Equal {
            return ordering;
        }
    }
    left.id.to_string().cmp(&right.id.to_string())
}

fn compare_task_field(left: &Task, right: &Task, field: SortField) -> Ordering {
    match field {
        SortField::Manual => left.sort_order.cmp(&right.sort_order),
        SortField::Priority => left.priority.cmp(&right.priority),
        SortField::Due => compare_task_due(&left.due, &right.due),
        SortField::UpdatedAt => left.updated_at.cmp(&right.updated_at),
        SortField::CreatedAt => left.created_at.cmp(&right.created_at),
        SortField::Title => left.title.to_lowercase().cmp(&right.title.to_lowercase()),
    }
}

fn compare_task_due(left: &Due, right: &Due) -> Ordering {
    match (left, right) {
        (Due::None, Due::None) => Ordering::Equal,
        (Due::None, _) => Ordering::Greater,
        (_, Due::None) => Ordering::Less,
        (Due::Date(left), Due::Date(right)) => left.cmp(right),
        (Due::DateTime(left), Due::DateTime(right)) => left.cmp(right),
        (Due::Date(left), Due::DateTime(right)) => left.cmp(
            &right
                .to_offset(UtcOffset::current_local_offset().unwrap_or(UtcOffset::UTC))
                .date(),
        ),
        (Due::DateTime(left), Due::Date(right)) => left
            .to_offset(UtcOffset::current_local_offset().unwrap_or(UtcOffset::UTC))
            .date()
            .cmp(right),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
