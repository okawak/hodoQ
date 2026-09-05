//! Saved-view editing and composition of live filters with domain queries.
use gpui::{
    AnyElement, Context, IntoElement, ParentElement as _, SharedString, Styled as _, Window, div,
    prelude::FluentBuilder as _, px,
};
use gpui_component::{Selectable as _, Sizable as _, button::Button, input::Input};
use time::{Date, OffsetDateTime, UtcOffset, macros::format_description};

use crate::domain::{
    DueScope, GroupBy, Priority, SavedBaseView, SavedView, SavedViewId, SortDirection, SortField,
    SortSpec, Task, TaskFilter, TaskStatus, ViewKind,
};

use super::theme;

use super::due::date_to_filter_datetime;
use super::{SmartView, Workspace, normalized_statuses};
use crate::domain::task_query::{TaskQuery, compare_tasks};

impl Workspace {
    pub(super) fn update_active_saved_view(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let SmartView::Saved(id) = self.active_view else {
            return;
        };
        let query = self.search_input.read(cx).value().to_string();
        let name = self.view_name_input.read(cx).value().trim().to_owned();
        if !name.is_empty() {
            if name.chars().count() > 100 {
                self.error_message = Some("保存ビュー名は1〜100文字で入力してください".to_owned());
                cx.notify();
                return;
            }
            if self
                .saved_views
                .iter()
                .any(|view| view.id != id && view.name.eq_ignore_ascii_case(name.as_str()))
            {
                self.error_message = Some("同じ名前の保存済みビューが既にあります".to_owned());
                cx.notify();
                return;
            }
        }
        let Some(index) = self.saved_views.iter().position(|view| view.id == id) else {
            return;
        };
        let before = self.saved_views[index].clone();
        let view = &mut self.saved_views[index];
        if !name.is_empty() {
            view.name = name;
        }
        view.view_kind = self.view_kind;
        view.filter.query = query;
        view.filter.statuses = self.filter_statuses.iter().copied().collect();
        view.filter.priorities = self.filter_priorities.iter().copied().collect();
        view.filter.project_ids = self.filter_projects.iter().copied().collect();
        view.filter.unassigned_project = self.filter_unassigned_project;
        view.filter.tag_ids = self.filter_tags.iter().copied().collect();
        view.filter.match_all_tags = self.filter_match_all_tags;
        view.filter.due_scope = self.filter_due;
        view.filter.due_from = self.filter_due_from.map(date_to_filter_datetime);
        view.filter.due_to = self.filter_due_to.map(date_to_filter_datetime);
        view.sort = self.sort.clone();
        view.group_by = self.group_by;
        view.updated_at = OffsetDateTime::now_utc();
        if let Err(error) = self.worker.save_view(view.clone()) {
            self.saved_views[index] = before;
            self.set_error(error);
            return;
        }
        self.view_name_input
            .update(cx, |state, cx| state.set_value("", window, cx));
        self.status_message = "保存済みビューを更新しました".to_owned();
        cx.notify();
    }

    pub(super) fn save_current_view(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.view_name_input.read(cx).value().to_string();
        if name.trim().is_empty() || name.chars().count() > 100 {
            self.error_message = Some("保存ビュー名は1〜100文字で入力してください".to_owned());
            cx.notify();
            return;
        }
        if self
            .saved_views
            .iter()
            .any(|view| view.name.eq_ignore_ascii_case(name.trim()))
        {
            self.error_message = Some("同じ名前の保存済みビューが既にあります".to_owned());
            cx.notify();
            return;
        }
        let now = OffsetDateTime::now_utc();
        let view = SavedView {
            id: SavedViewId::new(),
            name: name.trim().to_owned(),
            view_kind: self.view_kind,
            filter: TaskFilter {
                base_view: self.saved_base_view(),
                query: self.search_input.read(cx).value().to_string(),
                statuses: self.filter_statuses.iter().copied().collect(),
                priorities: self.filter_priorities.iter().copied().collect(),
                project_ids: self.filter_projects.iter().copied().collect(),
                unassigned_project: self.filter_unassigned_project,
                tag_ids: self.filter_tags.iter().copied().collect(),
                match_all_tags: self.filter_match_all_tags,
                due_scope: self.filter_due,
                due_from: self.filter_due_from.map(date_to_filter_datetime),
                due_to: self.filter_due_to.map(date_to_filter_datetime),
                ..TaskFilter::default()
            },
            sort: self.sort.clone(),
            group_by: self.group_by,
            sort_order: self.saved_views.len() as i64,
            created_at: now,
            updated_at: now,
        };
        if let Err(error) = self.worker.save_view(view.clone()) {
            self.set_error(error);
            return;
        }
        self.saved_views.push(view);
        self.view_name_input
            .update(cx, |state, cx| state.set_value("", window, cx));
        cx.notify();
    }

    fn saved_base_view(&self) -> Option<SavedBaseView> {
        match self.active_view {
            SmartView::Today => Some(SavedBaseView::Today),
            SmartView::Upcoming => Some(SavedBaseView::Upcoming),
            SmartView::Overdue => Some(SavedBaseView::Overdue),
            SmartView::Undated => Some(SavedBaseView::Undated),
            SmartView::All => None,
            SmartView::Doing => Some(SavedBaseView::Doing),
            SmartView::Blocked => Some(SavedBaseView::Blocked),
            SmartView::Done => Some(SavedBaseView::Done),
            SmartView::Archived => Some(SavedBaseView::Archived),
            SmartView::Trash => Some(SavedBaseView::Trash),
            SmartView::Saved(id) => self
                .saved_views
                .iter()
                .find(|view| view.id == id)
                .and_then(|view| view.filter.base_view),
        }
    }

    pub(super) fn delete_saved_view(&mut self, id: SavedViewId, cx: &mut Context<Self>) {
        if let Err(error) = self.worker.delete_view(id) {
            self.set_error(error);
            return;
        }
        self.saved_views.retain(|view| view.id != id);
        if self.active_view == SmartView::Saved(id) {
            self.active_view = SmartView::All;
        }
        self.status_message = "保存済みビューを削除しました".to_owned();
        cx.notify();
    }

    fn toggle_status_filter(&mut self, status: TaskStatus, cx: &mut Context<Self>) {
        if !self.filter_statuses.insert(status) {
            self.filter_statuses.remove(&status);
        }
        cx.notify();
    }

    fn toggle_priority_filter(&mut self, priority: Priority, cx: &mut Context<Self>) {
        if !self.filter_priorities.insert(priority) {
            self.filter_priorities.remove(&priority);
        }
        cx.notify();
    }

    pub(super) fn set_filter_due_boundary(
        &mut self,
        from: bool,
        value: &str,
        cx: &mut Context<Self>,
    ) {
        let value = value.trim();
        let parsed = if value.is_empty() {
            Some(None)
        } else {
            Date::parse(value, format_description!("[year]-[month]-[day]"))
                .ok()
                .map(Some)
        };
        let Some(date) = parsed else {
            self.error_message = Some("納期範囲は YYYY-MM-DD で入力してください".to_owned());
            cx.notify();
            return;
        };
        if from {
            self.filter_due_from = date;
        } else {
            self.filter_due_to = date;
        }
        if self
            .filter_due_from
            .zip(self.filter_due_to)
            .is_some_and(|(from, to)| from > to)
        {
            self.error_message = Some("納期範囲の開始日は終了日以前にしてください".to_owned());
        } else {
            self.error_message = None;
        }
        cx.notify();
    }

    pub(super) fn visible_tasks(&self, cx: &Context<Self>) -> Vec<Task> {
        let query = self.search_input.read(cx).value().to_lowercase();
        let now = OffsetDateTime::now_utc();
        let offset = UtcOffset::current_local_offset().unwrap_or(UtcOffset::UTC);
        let saved_view = match self.active_view {
            SmartView::Saved(id) => {
                let Some(view) = self.saved_views.iter().find(|view| view.id == id) else {
                    return Vec::new();
                };
                Some(view)
            }
            _ => None,
        };
        let filter = TaskFilter {
            base_view: self.saved_base_view(),
            statuses: self.filter_statuses.iter().copied().collect(),
            priorities: self.filter_priorities.iter().copied().collect(),
            project_ids: self.filter_projects.iter().copied().collect(),
            unassigned_project: self.filter_unassigned_project,
            tag_ids: self.filter_tags.iter().copied().collect(),
            match_all_tags: self.filter_match_all_tags,
            due_scope: self.filter_due,
            due_from: self
                .filter_due_from
                .map(|date| date.midnight().assume_offset(offset)),
            due_to: self
                .filter_due_to
                .map(|date| date.midnight().assume_offset(offset)),
            include_archived: saved_view.is_some_and(|view| view.filter.include_archived),
            only_deleted: saved_view.is_some_and(|view| view.filter.only_deleted),
            ..Default::default()
        };
        let live_query = TaskQuery::new(&filter, now, offset);
        let saved_query = saved_view.map(|view| TaskQuery::new(&view.filter, now, offset));
        let mut tasks = self
            .tasks
            .iter()
            // Live text search historically preserves spaces; saved queries trim them.
            .filter(|task| {
                query.is_empty()
                    || task.title.to_lowercase().contains(query.as_str())
                    || task.memo.to_lowercase().contains(query.as_str())
            })
            .filter(|task| {
                live_query.matches(task)
                    && saved_query.as_ref().is_none_or(|query| query.matches(task))
            })
            .cloned()
            .collect::<Vec<_>>();
        tasks.sort_by(|left, right| compare_tasks(left, right, &self.sort, offset));
        if self.view_kind == ViewKind::List {
            self.order_list_tasks(&mut tasks);
        }
        tasks
    }

    pub(super) fn render_filter_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex()
            .items_center()
            .flex_wrap()
            .gap_2()
            .px_4()
            .pb_3()
            .child(div().text_color(theme::MUTED).child("状態"))
            .children(TaskStatus::ALL.into_iter().map(|status| {
                let entity = cx.entity();
                Button::new(SharedString::from(format!(
                    "filter-status-{}",
                    status.as_str()
                )))
                .small()
                .label(status.label())
                .selected(self.filter_statuses.contains(&status))
                .on_click(move |_, _, cx| {
                    entity.update(cx, |this, cx| this.toggle_status_filter(status, cx));
                })
            }))
            .child(div().ml_3().text_color(theme::MUTED).child("優先度"))
            .children(Priority::ALL.into_iter().map(|priority| {
                let entity = cx.entity();
                Button::new(SharedString::from(format!(
                    "filter-priority-{}",
                    priority.as_str()
                )))
                .small()
                .label(priority.label())
                .selected(self.filter_priorities.contains(&priority))
                .on_click(move |_, _, cx| {
                    entity.update(cx, |this, cx| {
                        this.toggle_priority_filter(priority, cx);
                    });
                })
            }))
            .child(div().ml_3().text_color(theme::MUTED).child("納期"))
            .children(
                [
                    (DueScope::Any, "すべて"),
                    (DueScope::Today, "今日"),
                    (DueScope::Upcoming, "今後7日"),
                    (DueScope::Overdue, "期限超過"),
                    (DueScope::Undated, "未定"),
                ]
                .into_iter()
                .map(|(scope, label)| {
                    let entity = cx.entity();
                    Button::new(SharedString::from(format!("filter-due-{label}")))
                        .small()
                        .label(label)
                        .selected(self.filter_due == scope)
                        .on_click(move |_, _, cx| {
                            entity.update(cx, |this, cx| {
                                this.filter_due = scope;
                                cx.notify();
                            });
                        })
                }),
            )
            .child(Input::new(&self.filter_due_from_input).small().w(px(150.0)))
            .child(Input::new(&self.filter_due_to_input).small().w(px(150.0)))
            .child(self.small_action_button(
                "apply-due-range",
                "範囲を適用",
                cx,
                |this, _, cx| {
                    let from = this.filter_due_from_input.read(cx).value().to_string();
                    let to = this.filter_due_to_input.read(cx).value().to_string();
                    this.set_filter_due_boundary(true, &from, cx);
                    this.set_filter_due_boundary(false, &to, cx);
                },
            ))
            .child(div().ml_3().text_color(theme::MUTED).child("並び替え"))
            .children(
                [
                    (SortField::Manual, "手動"),
                    (SortField::Priority, "優先度"),
                    (SortField::Due, "納期"),
                    (SortField::UpdatedAt, "更新日"),
                    (SortField::CreatedAt, "作成日"),
                    (SortField::Title, "タイトル"),
                ]
                .into_iter()
                .map(|(field, label)| {
                    let entity = cx.entity();
                    Button::new(SharedString::from(format!("sort-field-{label}")))
                        .small()
                        .label(label)
                        .selected(self.sort[0].field == field)
                        .on_click(move |_, _, cx| {
                            entity.update(cx, |this, cx| {
                                this.sort[0].field = field;
                                if this.sort.get(1).is_some_and(|sort| sort.field == field) {
                                    this.sort.truncate(1);
                                }
                                cx.notify();
                            });
                        })
                }),
            )
            .child({
                let entity = cx.entity();
                Button::new("sort-direction")
                    .small()
                    .label(match self.sort[0].direction {
                        SortDirection::Ascending => "昇順",
                        SortDirection::Descending => "降順",
                    })
                    .on_click(move |_, _, cx| {
                        entity.update(cx, |this, cx| {
                            this.sort[0].direction = match this.sort[0].direction {
                                SortDirection::Ascending => SortDirection::Descending,
                                SortDirection::Descending => SortDirection::Ascending,
                            };
                            cx.notify();
                        });
                    })
            })
            .child(div().ml_3().text_color(theme::MUTED).child("第2キー"))
            .child({
                let entity = cx.entity();
                Button::new("secondary-sort-none")
                    .small()
                    .label("なし")
                    .selected(self.sort.len() == 1)
                    .on_click(move |_, _, cx| {
                        entity.update(cx, |this, cx| {
                            this.sort.truncate(1);
                            cx.notify();
                        });
                    })
            })
            .children(
                [
                    (SortField::Priority, "優先度"),
                    (SortField::Due, "納期"),
                    (SortField::UpdatedAt, "更新日"),
                    (SortField::CreatedAt, "作成日"),
                    (SortField::Title, "タイトル"),
                ]
                .into_iter()
                .filter(|(field, _)| *field != self.sort[0].field)
                .map(|(field, label)| {
                    let entity = cx.entity();
                    Button::new(SharedString::from(format!("secondary-sort-{label}")))
                        .small()
                        .label(label)
                        .selected(self.sort.get(1).is_some_and(|sort| sort.field == field))
                        .on_click(move |_, _, cx| {
                            entity.update(cx, |this, cx| {
                                let secondary = SortSpec {
                                    field,
                                    direction: this
                                        .sort
                                        .get(1)
                                        .map(|sort| sort.direction)
                                        .unwrap_or(SortDirection::Ascending),
                                };
                                if this.sort.len() == 1 {
                                    this.sort.push(secondary);
                                } else {
                                    this.sort[1] = secondary;
                                }
                                cx.notify();
                            });
                        })
                }),
            )
            .when(self.sort.len() > 1, |panel| {
                panel.child({
                    let entity = cx.entity();
                    Button::new("secondary-sort-direction")
                        .small()
                        .label(match self.sort[1].direction {
                            SortDirection::Ascending => "第2: 昇順",
                            SortDirection::Descending => "第2: 降順",
                        })
                        .on_click(move |_, _, cx| {
                            entity.update(cx, |this, cx| {
                                this.sort[1].direction = match this.sort[1].direction {
                                    SortDirection::Ascending => SortDirection::Descending,
                                    SortDirection::Descending => SortDirection::Ascending,
                                };
                                cx.notify();
                            });
                        })
                })
            })
            .child(div().ml_3().text_color(theme::MUTED).child("グループ"))
            .children(
                [
                    (None, "なし"),
                    (Some(GroupBy::Status), "状態"),
                    (Some(GroupBy::Priority), "優先度"),
                    (Some(GroupBy::Due), "納期"),
                ]
                .into_iter()
                .map(|(group, label)| {
                    let entity = cx.entity();
                    Button::new(SharedString::from(format!("group-by-{label}")))
                        .small()
                        .label(label)
                        .selected(self.group_by == group)
                        .on_click(move |_, _, cx| {
                            entity.update(cx, |this, cx| {
                                this.group_by = group;
                                cx.notify();
                            });
                        })
                }),
            )
            .child(self.small_action_button(
                "clear-filters",
                "条件をクリア",
                cx,
                |this, window, cx| {
                    this.filter_statuses.clear();
                    this.filter_priorities.clear();
                    this.filter_projects.clear();
                    this.filter_unassigned_project = false;
                    this.filter_tags.clear();
                    this.filter_match_all_tags = false;
                    this.filter_due = DueScope::Any;
                    this.filter_due_from = None;
                    this.filter_due_to = None;
                    this.filter_due_from_input
                        .update(cx, |state, cx| state.set_value("", window, cx));
                    this.filter_due_to_input
                        .update(cx, |state, cx| state.set_value("", window, cx));
                    this.sort = vec![SortSpec::default()];
                    this.group_by = None;
                    cx.notify();
                },
            ))
            .into_any_element()
    }
}

impl Workspace {
    pub(super) fn activate_view(
        &mut self,
        view: SmartView,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.flush_pending_edits(cx) {
            return;
        }
        self.active_view = view;
        self.selected_task = None;
        self.new_task_draft = None;
        self.sync_management_inputs(view, window, cx);
        if let SmartView::Saved(id) = view
            && let Some(saved) = self
                .saved_views
                .iter()
                .find(|saved| saved.id == id)
                .cloned()
        {
            self.view_kind = saved.view_kind;
            self.filter_statuses = normalized_statuses(&saved.filter.statuses);
            self.filter_priorities = saved.filter.priorities.iter().copied().collect();
            self.filter_projects = saved.filter.project_ids.iter().copied().collect();
            self.filter_unassigned_project = saved.filter.unassigned_project;
            self.filter_tags = saved.filter.tag_ids.iter().copied().collect();
            self.filter_match_all_tags = saved.filter.match_all_tags;
            self.filter_due = saved.filter.due_scope;
            let offset = UtcOffset::current_local_offset().unwrap_or(UtcOffset::UTC);
            self.filter_due_from = saved
                .filter
                .due_from
                .map(|date| date.to_offset(offset).date());
            self.filter_due_to = saved
                .filter
                .due_to
                .map(|date| date.to_offset(offset).date());
            self.sort = if saved.sort.is_empty() {
                vec![SortSpec::default()]
            } else {
                saved.sort.iter().copied().take(2).collect()
            };
            self.group_by = saved.group_by;
            self.search_input.update(cx, |state, cx| {
                state.set_value(saved.filter.query, window, cx);
            });
            let due_from = self
                .filter_due_from
                .map(|date| date.to_string())
                .unwrap_or_default();
            let due_to = self
                .filter_due_to
                .map(|date| date.to_string())
                .unwrap_or_default();
            self.filter_due_from_input.update(cx, |state, cx| {
                state.set_value(due_from, window, cx);
            });
            self.filter_due_to_input.update(cx, |state, cx| {
                state.set_value(due_to, window, cx);
            });
        }
        cx.notify();
    }
}
