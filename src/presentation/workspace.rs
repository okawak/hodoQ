use std::{collections::HashSet, path::PathBuf, time::Duration as StdDuration};

use gpui::{
    AnyElement, AppContext as _, Context, Entity, FontWeight, IntoElement, ParentElement as _,
    Render, SharedString, Styled as _, Subscription, Timer, Window, div, px,
};
use gpui_component::{
    Sizable as _,
    button::Button,
    calendar::{CalendarEvent, CalendarState},
    input::{InputEvent, InputState},
    resizable::ResizableState,
};
use time::{Date, OffsetDateTime, UtcOffset};

use crate::{
    application::{AppDataSnapshot, ApplicationError, HistoryEntry, TaskApplication},
    domain::{
        DueScope, GroupBy, Priority, Project, ProjectId, SavedView, SavedViewId, SortSpec, Tag,
        TagId, Task, TaskId, TaskStatus, ViewKind,
    },
    infrastructure::{AppPaths, AppSettings, InstanceLock},
};

use super::theme;
use shell::{smart_view_from_setting, smart_view_setting};

mod board;
mod calendar;
mod commands;
mod data_management;
mod due;
mod due_control;
mod history;
mod preferences;
mod shell;
mod task_actions;
mod task_editor;
mod task_list;
mod task_query;
mod views;

use due::{due_is_today, parse_due};
use task_query::saved_view_is_available;

const CALENDAR_DAY_CELL_HEIGHT: f32 = 104.0;
const CALENDAR_WEEKDAY_HEIGHT: f32 = 28.0;
const CALENDAR_GRID_MIN_HEIGHT: f32 = CALENDAR_DAY_CELL_HEIGHT * 6.0 + CALENDAR_WEEKDAY_HEIGHT;

gpui::actions!(
    hodoq,
    [
        NewTaskAction,
        SearchAction,
        MoveUpAction,
        MoveDownAction,
        ToggleDoneAction,
        DeleteAction,
        CloseDetailAction,
        CommandPaletteAction,
        UndoAction,
        RedoAction,
        SelectAllAction
    ]
);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SmartView {
    Today,
    Upcoming,
    Overdue,
    Undated,
    All,
    Doing,
    Blocked,
    Done,
    Archived,
    Trash,
    Saved(SavedViewId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CalendarMode {
    Month,
    Agenda,
}

#[derive(Debug, Clone, Copy)]
enum PaletteCommand {
    NewTask,
    Search,
    Today,
    All,
    StatusTodo,
    StatusDoing,
    StatusBlocked,
    StatusDone,
    StatusArchived,
    PriorityNone,
    PriorityLow,
    PriorityMedium,
    PriorityHigh,
    Backup,
}

#[derive(Debug, Clone)]
enum PendingConfirmation {
    EmptyTrash,
    Restore(PathBuf),
    CloseSaveFailed,
}

#[derive(Debug, Clone)]
struct NewTaskDraft {
    status: TaskStatus,
    priority: Priority,
    progress: u8,
}

impl Default for NewTaskDraft {
    fn default() -> Self {
        Self {
            status: TaskStatus::Todo,
            priority: Priority::None,
            progress: 0,
        }
    }
}

#[derive(Clone)]
struct TaskDrag {
    id: TaskId,
    title: String,
}

impl Render for TaskDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .max_w(px(300.0))
            .px_3()
            .py_2()
            .rounded_md()
            .border_1()
            .border_color(theme::ACCENT)
            .bg(theme::SURFACE)
            .text_color(theme::TEXT)
            .child(self.title.clone())
    }
}

pub(super) struct Workspace {
    worker: TaskApplication,
    paths: AppPaths,
    settings: AppSettings,
    _instance_lock: InstanceLock,
    tasks: Vec<Task>,
    projects: Vec<Project>,
    tags: Vec<Tag>,
    saved_views: Vec<SavedView>,
    selected_task: Option<TaskId>,
    selection_anchor: Option<TaskId>,
    selected_tasks: HashSet<TaskId>,
    active_view: SmartView,
    view_kind: ViewKind,
    task_list_state: gpui::ListState,
    filter_statuses: HashSet<TaskStatus>,
    filter_priorities: HashSet<Priority>,
    filter_projects: HashSet<ProjectId>,
    filter_unassigned_project: bool,
    filter_tags: HashSet<TagId>,
    filter_match_all_tags: bool,
    filter_due: DueScope,
    filter_due_from: Option<Date>,
    filter_due_to: Option<Date>,
    sort: Vec<SortSpec>,
    group_by: Option<GroupBy>,
    calendar_mode: CalendarMode,
    calendar_month: Date,
    selection_mode: bool,
    new_task_draft: Option<NewTaskDraft>,
    show_more_menu: bool,
    show_all_smart_views: bool,
    show_saved_views: bool,
    show_filter_panel: bool,
    show_data_panel: bool,
    show_command_palette: bool,
    csv_with_bom: bool,
    pending_confirmation: Option<PendingConfirmation>,
    undo_stack: Vec<HistoryEntry>,
    redo_stack: Vec<HistoryEntry>,
    title_revision: u64,
    memo_revision: u64,
    pending_title: Option<(TaskId, String)>,
    pending_memo: Option<(TaskId, String)>,
    allow_close: bool,
    close_save_completed: bool,
    discard_unsaved_on_close: bool,
    status_message: String,
    error_message: Option<String>,
    search_input: Entity<InputState>,
    command_input: Entity<InputState>,
    title_input: Entity<InputState>,
    memo_input: Entity<InputState>,
    due_input: Entity<InputState>,
    due_calendar: Entity<CalendarState>,
    due_popover_open: bool,
    due_input_error: Option<String>,
    due_focus: gpui::FocusHandle,
    due_input_bounds: Option<gpui::Bounds<gpui::Pixels>>,
    show_due_times: bool,
    progress_input: Entity<InputState>,
    view_name_input: Entity<InputState>,
    bulk_due_input: Entity<InputState>,
    filter_due_from_input: Entity<InputState>,
    filter_due_to_input: Entity<InputState>,
    restore_path_input: Entity<InputState>,
    sidebar_resize_state: Entity<ResizableState>,
    _subscriptions: Vec<Subscription>,
}

impl Workspace {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn new(
        worker: TaskApplication,
        snapshot: AppDataSnapshot,
        paths: AppPaths,
        settings: AppSettings,
        instance_lock: InstanceLock,
        first_run: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("タイトル・メモを検索"));
        let command_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("操作を検索（例: 今日、完了、バックアップ）")
        });
        let title_input = cx.new(|cx| InputState::new(window, cx).placeholder("タイトル"));
        let memo_input = cx.new(|cx| {
            InputState::new(window, cx)
                .multi_line(true)
                .placeholder("メモ")
        });
        let due_input = cx.new(|cx| InputState::new(window, cx).placeholder("日付を入力・選択"));
        let due_calendar = cx.new(|cx| CalendarState::new(window, cx));
        let due_focus = cx.focus_handle();
        let progress_input = cx.new(|cx| InputState::new(window, cx).placeholder("進捗 0〜100"));
        let view_name_input = cx.new(|cx| InputState::new(window, cx).placeholder("保存ビュー名"));
        let bulk_due_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("一括納期: YYYY-MM-DD または日時"));
        let filter_due_from_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("開始 YYYY-MM-DD"));
        let filter_due_to_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("終了 YYYY-MM-DD"));
        let restore_path_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("復元するSQLiteバックアップのファイルパス")
        });
        let sidebar_resize_state = cx.new(|_| ResizableState::default());

        let _subscriptions = vec![
            cx.on_focus_out(&due_focus, window, |this, _, _, cx| {
                this.dismiss_due_popover(cx);
            }),
            cx.subscribe(&search_input, |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }),
            cx.subscribe(&command_input, |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }),
            cx.subscribe(&title_input, |this, state, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.schedule_title_save(state.read(cx).value().to_string(), cx);
                }
            }),
            cx.subscribe(&memo_input, |this, state, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.schedule_memo_save(state.read(cx).value().to_string(), cx);
                }
            }),
            cx.subscribe_in(
                &due_calendar,
                window,
                |this, _, event: &CalendarEvent, window, cx| {
                    let CalendarEvent::Selected(date) = event;
                    this.select_due_date(*date, window, cx);
                },
            ),
            cx.subscribe_in(
                &due_input,
                window,
                |this, state, event: &InputEvent, window, cx| {
                    let value = state.read(cx).value().to_string();
                    match event {
                        InputEvent::PressEnter { .. } => {
                            this.update_due_from_input(&value, window, cx);
                            if parse_due(&value).is_ok() {
                                this.dismiss_due_popover(cx);
                            }
                        }
                        InputEvent::Change => this.sync_due_picker_from_input(&value, window, cx),
                        _ => {}
                    }
                },
            ),
            cx.subscribe_in(
                &progress_input,
                window,
                |this, state, event: &InputEvent, _window, cx| {
                    if matches!(event, InputEvent::PressEnter { .. }) {
                        let value = state.read(cx).value().to_string();
                        match value.trim().parse::<u8>() {
                            Ok(progress) if progress <= 100 => {
                                if let Some(id) = this.selected_task {
                                    this.set_task_progress(id, progress, cx);
                                }
                                this.error_message = None;
                            }
                            _ => {
                                this.error_message =
                                    Some("進捗は0〜100の整数で入力してください".to_owned());
                                cx.notify();
                            }
                        }
                    }
                },
            ),
            cx.subscribe_in(
                &filter_due_from_input,
                window,
                |this, state, event: &InputEvent, _window, cx| {
                    if matches!(event, InputEvent::PressEnter { .. }) {
                        this.set_filter_due_boundary(true, state.read(cx).value().as_str(), cx);
                    }
                },
            ),
            cx.subscribe_in(
                &filter_due_to_input,
                window,
                |this, state, event: &InputEvent, _window, cx| {
                    if matches!(event, InputEvent::PressEnter { .. }) {
                        this.set_filter_due_boundary(false, state.read(cx).value().as_str(), cx);
                    }
                },
            ),
            cx.subscribe_in(
                &view_name_input,
                window,
                |this, _, event: &InputEvent, window, cx| {
                    if matches!(event, InputEvent::PressEnter { .. }) {
                        if matches!(this.active_view, SmartView::Saved(_)) {
                            this.update_active_saved_view(window, cx);
                        } else {
                            this.save_current_view(window, cx);
                        }
                    }
                },
            ),
            cx.subscribe_in(
                &restore_path_input,
                window,
                |this, state, event: &InputEvent, _window, cx| {
                    if matches!(event, InputEvent::PressEnter { .. }) {
                        this.request_restore_path(state.read(cx).value().as_str(), cx);
                    }
                },
            ),
        ];

        let initial_view_kind = settings.view_kind;
        let mut initial_sort = if settings.sort.is_empty() {
            vec![SortSpec::default()]
        } else {
            settings.sort.iter().copied().take(2).collect()
        };
        let mut initial_group_by = settings.group_by.filter(|group| *group != GroupBy::Project);
        let offset = UtcOffset::current_local_offset().unwrap_or(UtcOffset::UTC);
        let today = OffsetDateTime::now_utc().to_offset(offset).date();
        let calendar_month =
            Date::from_calendar_date(today.year(), today.month(), 1).unwrap_or(today);
        let mut initial_active_view = if first_run
            && !snapshot.tasks.iter().any(|task| {
                task.deleted_at.is_none()
                    && task.status != TaskStatus::Archived
                    && due_is_today(&task.due, today, offset)
            }) {
            SmartView::All
        } else {
            smart_view_from_setting(&settings.active_view)
        };
        initial_active_view = match initial_active_view {
            SmartView::Saved(id)
                if !snapshot
                    .saved_views
                    .iter()
                    .any(|view| view.id == id && saved_view_is_available(view)) =>
            {
                SmartView::All
            }
            view => view,
        };
        let mut initial_filter_statuses = HashSet::new();
        let mut initial_filter_priorities = HashSet::new();
        let mut initial_filter_projects = HashSet::new();
        let mut initial_filter_unassigned_project = false;
        let mut initial_filter_tags = HashSet::new();
        let mut initial_filter_match_all_tags = false;
        let mut initial_filter_due = DueScope::Any;
        let mut initial_filter_due_from = None;
        let mut initial_filter_due_to = None;
        if let SmartView::Saved(id) = initial_active_view
            && let Some(saved) = snapshot.saved_views.iter().find(|view| view.id == id)
        {
            // Restore the last chosen presentation, not the saved view's preset.
            // Explicitly opening a saved view still applies its preset in activate_view.
            initial_sort = if saved.sort.is_empty() {
                vec![SortSpec::default()]
            } else {
                saved.sort.iter().copied().take(2).collect()
            };
            initial_group_by = saved.group_by;
            initial_filter_statuses = normalized_statuses(&saved.filter.statuses);
            initial_filter_priorities = saved.filter.priorities.iter().copied().collect();
            initial_filter_projects = saved.filter.project_ids.iter().copied().collect();
            initial_filter_unassigned_project = saved.filter.unassigned_project;
            initial_filter_tags = saved.filter.tag_ids.iter().copied().collect();
            initial_filter_match_all_tags = saved.filter.match_all_tags;
            initial_filter_due = saved.filter.due_scope;
            initial_filter_due_from = saved
                .filter
                .due_from
                .map(|value| value.to_offset(offset).date());
            initial_filter_due_to = saved
                .filter
                .due_to
                .map(|value| value.to_offset(offset).date());
            search_input.update(cx, |state, cx| {
                state.set_value(saved.filter.query.clone(), window, cx);
            });
            filter_due_from_input.update(cx, |state, cx| {
                state.set_value(
                    initial_filter_due_from
                        .map(|date| date.to_string())
                        .unwrap_or_default(),
                    window,
                    cx,
                );
            });
            filter_due_to_input.update(cx, |state, cx| {
                state.set_value(
                    initial_filter_due_to
                        .map(|date| date.to_string())
                        .unwrap_or_default(),
                    window,
                    cx,
                );
            });
        }
        if let SmartView::Saved(id) = initial_active_view
            && let Some(saved) = snapshot.saved_views.iter().find(|view| view.id == id)
        {
            view_name_input.update(cx, |state, cx| {
                state.set_value(saved.name.clone(), window, cx);
            });
        }
        let startup_warning = worker.startup_warning().map(str::to_owned);
        let read_only = worker.is_read_only();

        let workspace = Self {
            worker,
            paths,
            settings,
            _instance_lock: instance_lock,
            tasks: snapshot.tasks,
            projects: snapshot.projects,
            tags: snapshot.tags,
            saved_views: snapshot.saved_views,
            selected_task: None,
            selection_anchor: None,
            selected_tasks: HashSet::new(),
            active_view: initial_active_view,
            view_kind: initial_view_kind,
            task_list_state: gpui::ListState::new(0, gpui::ListAlignment::Top, px(200.0)),
            filter_statuses: initial_filter_statuses,
            filter_priorities: initial_filter_priorities,
            filter_projects: initial_filter_projects,
            filter_unassigned_project: initial_filter_unassigned_project,
            filter_tags: initial_filter_tags,
            filter_match_all_tags: initial_filter_match_all_tags,
            filter_due: initial_filter_due,
            filter_due_from: initial_filter_due_from,
            filter_due_to: initial_filter_due_to,
            sort: initial_sort,
            group_by: initial_group_by,
            calendar_mode: CalendarMode::Month,
            calendar_month,
            selection_mode: false,
            new_task_draft: None,
            show_more_menu: false,
            show_all_smart_views: false,
            show_saved_views: matches!(initial_active_view, SmartView::Saved(_)),
            show_filter_panel: false,
            show_data_panel: false,
            show_command_palette: false,
            csv_with_bom: true,
            pending_confirmation: None,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            title_revision: 0,
            memo_revision: 0,
            pending_title: None,
            pending_memo: None,
            allow_close: false,
            close_save_completed: false,
            discard_unsaved_on_close: false,
            status_message: if read_only {
                "読み取り専用".to_owned()
            } else {
                "保存済み".to_owned()
            },
            error_message: startup_warning,
            search_input,
            command_input,
            title_input,
            memo_input,
            due_input,
            due_calendar,
            due_popover_open: false,
            due_input_error: None,
            due_focus,
            due_input_bounds: None,
            show_due_times: false,
            progress_input,
            view_name_input,
            bulk_due_input,
            filter_due_from_input,
            filter_due_to_input,
            restore_path_input,
            sidebar_resize_state,
            _subscriptions,
        };
        cx.spawn(async move |this, cx| {
            loop {
                Timer::after(StdDuration::from_secs(24 * 60 * 60)).await;
                let Some(this) = this.upgrade() else {
                    break;
                };
                if this
                    .update(cx, |this, cx| this.purge_expired_trash_from_ui(cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        workspace
    }

    fn set_error(&mut self, error: ApplicationError) {
        while self.worker.take_error().is_some() {}
        self.error_message = Some(error.to_string());
        self.status_message = if self.worker.is_read_only() {
            "読み取り専用".to_owned()
        } else {
            "保存失敗".to_owned()
        };
    }

    fn selected_task(&self) -> Option<&Task> {
        let id = self.selected_task?;
        self.tasks.iter().find(|task| task.id == id)
    }

    fn small_action_button(
        &self,
        id: &'static str,
        label: &'static str,
        cx: &mut Context<Self>,
        handler: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) -> AnyElement {
        let entity = cx.entity();
        Button::new(id)
            .small()
            .label(label)
            .on_click(move |_, window, cx| {
                entity.update(cx, |this, cx| handler(this, window, cx));
            })
            .into_any_element()
    }

    fn render_content(&self, cx: &mut Context<Self>) -> AnyElement {
        let tasks = self.visible_tasks(cx);
        let view = match self.view_kind {
            ViewKind::List => self.render_list(tasks, cx),
            ViewKind::Board => self.render_board(tasks, cx),
            ViewKind::Calendar => self.render_calendar(tasks, cx),
        };
        div()
            .flex()
            .flex_col()
            .flex_1()
            .h_full()
            .min_h_0()
            .child(view)
            .into_any_element()
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        if self.discard_unsaved_on_close {
            let _ = self.worker.flush();
            self.update_persisted_settings_fields();
            if let Err(error) = self.settings.save(&self.paths.settings) {
                tracing::error!(%error, "failed to save settings while closing");
            }
        } else if !self.close_save_completed
            && let Err(error) = self.persist_before_close()
        {
            tracing::error!(%error, "failed to persist data while dropping workspace");
        }
    }
}

fn section_label(label: impl Into<SharedString>) -> impl IntoElement {
    div()
        .mt_3()
        .mb_1()
        .text_size(px(12.0))
        .font_weight(FontWeight::BOLD)
        .text_color(theme::MUTED)
        .child(label.into())
}

fn labeled_input(label: &'static str, input: impl IntoElement) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .text_size(px(12.0))
                .font_weight(FontWeight::BOLD)
                .text_color(theme::MUTED)
                .child(label),
        )
        .child(input)
}

fn priority_color(priority: Priority) -> gpui::Rgba {
    match priority {
        Priority::None => theme::MUTED,
        Priority::Low => theme::SUCCESS,
        Priority::Medium => theme::WARNING,
        Priority::High => theme::DANGER,
    }
}

fn normalized_statuses(statuses: &[TaskStatus]) -> HashSet<TaskStatus> {
    statuses
        .iter()
        .map(|status| match status {
            TaskStatus::Inbox => TaskStatus::Todo,
            status => *status,
        })
        .collect()
}

#[cfg(test)]
mod tests;
