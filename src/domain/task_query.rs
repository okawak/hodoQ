//! Pure filtering and ordering shared by persistence and presentation.
//! Callers capture the clock and local offset once for a whole query.
use super::{
    Due, DueScope, SavedBaseView, SortDirection, SortField, SortSpec, Task, TaskFilter, TaskStatus,
};
use std::cmp::Ordering;
use time::{OffsetDateTime, UtcOffset};

pub(crate) struct TaskQuery<'a> {
    filter: &'a TaskFilter,
    query: String,
    now: OffsetDateTime,
    offset: UtcOffset,
}

impl<'a> TaskQuery<'a> {
    pub(crate) fn new(filter: &'a TaskFilter, now: OffsetDateTime, offset: UtcOffset) -> Self {
        Self {
            filter,
            query: filter.query.trim().to_lowercase(),
            now,
            offset,
        }
    }

    pub(crate) fn matches(&self, task: &Task) -> bool {
        let filter = self.filter;
        let now = self.now;
        let offset = self.offset;
        let visibility_matches = match filter.base_view {
            Some(SavedBaseView::Trash) => task.deleted_at.is_some(),
            _ if filter.only_deleted => task.deleted_at.is_some(),
            _ => task.deleted_at.is_none(),
        };
        if !visibility_matches {
            return false;
        }
        if !filter.include_archived
            && !filter.only_deleted
            && filter.base_view != Some(SavedBaseView::Archived)
            && filter.base_view != Some(SavedBaseView::Trash)
            && task.status == TaskStatus::Archived
        {
            return false;
        }
        let query = &self.query;
        if !query.is_empty()
            && !task.title.to_lowercase().contains(query)
            && !task.memo.to_lowercase().contains(query)
        {
            return false;
        }
        if !status_filter_matches(&filter.statuses, task.status) {
            return false;
        }
        if !filter.priorities.is_empty() && !filter.priorities.contains(&task.priority) {
            return false;
        }
        if !filter.project_ids.is_empty() || filter.unassigned_project {
            let matched = task.project_id.map_or(filter.unassigned_project, |id| {
                filter.project_ids.contains(&id)
            });
            if !matched {
                return false;
            }
        }
        if !filter.tag_ids.is_empty() {
            let matched = if filter.match_all_tags {
                filter.tag_ids.iter().all(|id| task.tag_ids.contains(id))
            } else {
                filter.tag_ids.iter().any(|id| task.tag_ids.contains(id))
            };
            if !matched {
                return false;
            }
        }
        let today = now.to_offset(offset).date();
        let due_date = match &task.due {
            Due::None => None,
            Due::Date(date) => Some(*date),
            Due::DateTime(date_time) => Some(date_time.to_offset(offset).date()),
        };
        let smart_view_matches = match filter.base_view {
            None | Some(SavedBaseView::Trash) => true,
            Some(SavedBaseView::Inbox) => task.status == TaskStatus::Todo,
            Some(SavedBaseView::Today) => due_date == Some(today),
            Some(SavedBaseView::Upcoming) => due_date
                .is_some_and(|date| date >= today && date <= today + time::Duration::days(7)),
            Some(SavedBaseView::Overdue) => {
                task.status != TaskStatus::Done && task.due.is_overdue(now, today)
            }
            Some(SavedBaseView::Undated) => due_date.is_none(),
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
        let due_scope_matches = match filter.due_scope {
            DueScope::Any => true,
            DueScope::Undated => due_date.is_none(),
            DueScope::Today => due_date == Some(today),
            DueScope::Upcoming => due_date
                .is_some_and(|date| date >= today && date <= today + time::Duration::days(7)),
            DueScope::Overdue => task.status != TaskStatus::Done && task.due.is_overdue(now, today),
        };
        if !due_scope_matches {
            return false;
        }
        if let Some(from) = filter.due_from {
            let from_date = from.to_offset(offset).date();
            if due_date.is_none_or(|date| date < from_date) {
                return false;
            }
        }
        if let Some(to) = filter.due_to {
            let to_date = to.to_offset(offset).date();
            if due_date.is_none_or(|date| date > to_date) {
                return false;
            }
        }
        true
    }
}

pub(crate) fn status_filter_matches(statuses: &[TaskStatus], status: TaskStatus) -> bool {
    statuses.is_empty()
        || statuses.contains(&status)
        || (status == TaskStatus::Todo && statuses.contains(&TaskStatus::Inbox))
}

pub(crate) fn compare_tasks(
    left: &Task,
    right: &Task,
    sort: &[SortSpec],
    offset: UtcOffset,
) -> Ordering {
    for sort in sort {
        let ordering = compare_task_field(left, right, sort.field, offset);
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

fn compare_task_field(left: &Task, right: &Task, field: SortField, offset: UtcOffset) -> Ordering {
    match field {
        SortField::Manual => left.sort_order.cmp(&right.sort_order),
        SortField::Priority => left.priority.cmp(&right.priority),
        SortField::Due => compare_task_due(&left.due, &right.due, offset),
        SortField::UpdatedAt => left.updated_at.cmp(&right.updated_at),
        SortField::CreatedAt => left.created_at.cmp(&right.created_at),
        SortField::Title => left.title.to_lowercase().cmp(&right.title.to_lowercase()),
    }
}

fn compare_task_due(left: &Due, right: &Due, offset: UtcOffset) -> Ordering {
    // Compare every dated value on the same timeline. Treat date-only deadlines
    // as the end of that local day, including when the other value has a time.
    let key = |due: &Due| match due {
        Due::None => None,
        Due::Date(date) => Some(date.with_time(time::macros::time!(23:59:59.999_999_999))),
        Due::DateTime(value) => {
            let local = value.to_offset(offset);
            Some(local.date().with_time(local.time()))
        }
    };
    match (key(left), key(right)) {
        (None, None) => Ordering::Equal,
        (None, _) => Ordering::Greater,
        (_, None) => Ordering::Less,
        (Some(left), Some(right)) => left.cmp(&right),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::{date, datetime, offset};

    #[test]
    fn local_midnight_and_due_boundaries_use_the_supplied_clock() {
        let now = datetime!(2026-09-05 15:00 UTC);
        let offset = offset!(+9);
        let mut task = Task::new("tomorrow in UTC", now).unwrap();
        task.due = Due::DateTime(now);
        let filter = TaskFilter {
            due_scope: DueScope::Today,
            due_from: Some(now),
            due_to: Some(now),
            ..Default::default()
        };
        assert!(TaskQuery::new(&filter, now, offset).matches(&task));
        task.due = Due::Date(date!(2026 - 09 - 05));
        assert!(!TaskQuery::new(&filter, now, offset).matches(&task));
        task.due = Due::None;
        assert!(!TaskQuery::new(&filter, now, offset).matches(&task));
    }

    #[test]
    fn upcoming_includes_seventh_day_but_not_eighth() {
        let now = datetime!(2026-09-05 12:00 UTC);
        let filter = TaskFilter {
            due_scope: DueScope::Upcoming,
            ..Default::default()
        };
        let query = TaskQuery::new(&filter, now, UtcOffset::UTC);
        let mut task = Task::new("deadline", now).unwrap();
        for (date, expected) in [
            (date!(2026 - 09 - 04), false),
            (date!(2026 - 09 - 05), true),
            (date!(2026 - 09 - 12), true),
            (date!(2026 - 09 - 13), false),
        ] {
            task.due = Due::Date(date);
            assert_eq!(query.matches(&task), expected);
        }
    }

    #[test]
    fn trash_archive_and_legacy_status_filters_preserve_scope() {
        let now = datetime!(2026-09-05 12:00 UTC);
        let mut task = Task::new("  Needle  ", now).unwrap();
        let mut filter = TaskFilter {
            query: " NEEDLE ".into(),
            statuses: vec![TaskStatus::Inbox],
            ..Default::default()
        };
        assert!(TaskQuery::new(&filter, now, UtcOffset::UTC).matches(&task));
        task.status = TaskStatus::Archived;
        filter.statuses.clear();
        assert!(!TaskQuery::new(&filter, now, UtcOffset::UTC).matches(&task));
        filter.base_view = Some(SavedBaseView::Archived);
        assert!(TaskQuery::new(&filter, now, UtcOffset::UTC).matches(&task));
        task.deleted_at = Some(now);
        assert!(!TaskQuery::new(&filter, now, UtcOffset::UTC).matches(&task));
        filter.base_view = Some(SavedBaseView::Trash);
        assert!(TaskQuery::new(&filter, now, UtcOffset::UTC).matches(&task));
    }

    #[test]
    fn due_order_uses_local_date_and_direction() {
        let now = datetime!(2026-09-05 15:00 UTC);
        let mut left = Task::new("left", now).unwrap();
        let mut right = left.clone();
        left.due = Due::Date(date!(2026 - 09 - 06));
        right.due = Due::DateTime(now);
        let sort = [SortSpec {
            field: SortField::Due,
            direction: SortDirection::Ascending,
        }];
        assert_eq!(
            compare_tasks(&left, &right, &sort, offset!(+9)),
            Ordering::Greater
        );
        assert_eq!(
            compare_tasks(&left, &right, &sort, UtcOffset::UTC),
            Ordering::Greater
        );
        // In UTC the datetime is September 5; in +09 it is September 6.
        left.due = Due::Date(date!(2026 - 09 - 05));
        assert_eq!(
            compare_tasks(&left, &right, &sort, UtcOffset::UTC),
            Ordering::Greater
        );
        assert_eq!(
            compare_tasks(&left, &right, &sort, offset!(+9)),
            Ordering::Less
        );
        right.due = Due::None;
        assert_eq!(
            compare_tasks(&left, &right, &sort, offset!(+9)),
            Ordering::Less
        );
        let descending = [SortSpec {
            direction: SortDirection::Descending,
            ..sort[0]
        }];
        assert_eq!(
            compare_tasks(&left, &right, &descending, offset!(+9)),
            Ordering::Greater
        );
    }
    #[test]
    fn mixed_due_comparison_is_transitive() {
        let now = datetime!(2026-09-05 0:00 UTC);
        let dues = [
            Due::DateTime(now + time::Duration::hours(9)),
            Due::Date(date!(2026 - 09 - 05)),
            Due::DateTime(now + time::Duration::hours(18)),
            Due::None,
            Due::Date(date!(2026 - 09 - 06)),
        ];
        let tasks = dues
            .into_iter()
            .enumerate()
            .map(|(index, due)| {
                let mut task = Task::new("task", now).unwrap();
                task.id = format!("00000000-0000-0000-0000-{:012}", 10 - index)
                    .parse()
                    .unwrap();
                task.due = due;
                task
            })
            .collect::<Vec<_>>();
        for offset in [UtcOffset::UTC, offset!(+9), offset!(-7)] {
            for direction in [SortDirection::Ascending, SortDirection::Descending] {
                let sort = [SortSpec {
                    field: SortField::Due,
                    direction,
                }];
                for a in &tasks {
                    for b in &tasks {
                        assert_eq!(
                            compare_tasks(a, b, &sort, offset),
                            compare_tasks(b, a, &sort, offset).reverse()
                        );
                        for c in &tasks {
                            if compare_tasks(a, b, &sort, offset).is_gt()
                                && compare_tasks(b, c, &sort, offset).is_gt()
                            {
                                assert!(
                                    compare_tasks(a, c, &sort, offset).is_gt(),
                                    "non-transitive comparison: {:?}, {:?}, {:?}",
                                    a.due,
                                    b.due,
                                    c.due
                                );
                            }
                        }
                    }
                }
            }
        }
    }
}
