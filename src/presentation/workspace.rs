use std::{collections::HashSet, fs, path::PathBuf, time::Duration as StdDuration};

use gpui::{
    AnyElement, AppContext as _, Context, Entity, Focusable as _, FontWeight,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, Timer, Window, div,
    prelude::FluentBuilder as _, px,
};
use gpui_component::{
    Disableable as _, Selectable as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    calendar::Date as PickerDate,
    calendar::{CalendarEvent, CalendarState},
    checkbox::Checkbox,
    input::{Input, InputEvent, InputState},
    menu::{ContextMenuExt as _, PopupMenuItem},
    progress::Progress,
    resizable::{ResizableState, h_resizable, resizable_panel},
    scroll::ScrollableElement as _,
};
use time::{Date, OffsetDateTime, UtcOffset, macros::format_description};

use crate::{
    application::{HistoryEntry, TaskApplication},
    domain::{
        Due, DueScope, GroupBy, Priority, Project, ProjectId, SavedBaseView, SavedView,
        SavedViewId, SortDirection, SortField, SortSpec, Tag, TagId, Task, TaskFilter, TaskId,
        TaskStatus, ViewKind,
    },
    infrastructure::{AppDataSnapshot, AppPaths, AppSettings, InstanceLock, RepositoryError},
};

use super::theme;

mod due;
mod due_control;
mod task_editor;
mod task_list;
mod task_query;

use crate::domain::task_query::{TaskQuery, compare_tasks};
use due::*;
use task_query::*;

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

        let mut initial_view_kind = settings.view_kind;
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
            initial_view_kind = saved.view_kind;
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

    fn push_task_history(&mut self, changes: Vec<(Option<Task>, Option<Task>)>) {
        if changes.is_empty() {
            return;
        }
        self.push_history(HistoryEntry::tasks(changes));
    }

    fn push_history(&mut self, history: HistoryEntry) {
        self.undo_stack.push(history);
        if self.undo_stack.len() > 50 {
            self.undo_stack.remove(0);
        }
        self.redo_stack.clear();
    }

    fn apply_history_state(&mut self, history: &HistoryEntry, use_after: bool) {
        for (before, after) in &history.task_changes {
            let state = if use_after { after } else { before };
            let id = before
                .as_ref()
                .or(after.as_ref())
                .map(|task| task.id)
                .expect("task history entry must have a task");
            match state {
                Some(task) => {
                    if let Some(existing) = self.tasks.iter_mut().find(|item| item.id == id) {
                        *existing = task.clone();
                    } else {
                        self.tasks.push(task.clone());
                    }
                }
                None => self.tasks.retain(|task| task.id != id),
            }
        }
        for (before, after) in &history.project_changes {
            let state = if use_after { after } else { before };
            let id = before
                .as_ref()
                .or(after.as_ref())
                .map(|project| project.id)
                .expect("project history entry must have a project");
            match state {
                Some(project) => {
                    if let Some(existing) = self.projects.iter_mut().find(|item| item.id == id) {
                        *existing = project.clone();
                    } else {
                        self.projects.push(project.clone());
                    }
                }
                None => self.projects.retain(|project| project.id != id),
            }
        }
        for (before, after) in &history.tag_changes {
            let state = if use_after { after } else { before };
            let id = before
                .as_ref()
                .or(after.as_ref())
                .map(|tag| tag.id)
                .expect("tag history entry must have a tag");
            match state {
                Some(tag) => {
                    if let Some(existing) = self.tags.iter_mut().find(|item| item.id == id) {
                        *existing = tag.clone();
                    } else {
                        self.tags.push(tag.clone());
                    }
                }
                None => self.tags.retain(|tag| tag.id != id),
            }
        }
    }

    fn persist_history_state(
        &self,
        history: &HistoryEntry,
        use_after: bool,
    ) -> Result<(), RepositoryError> {
        let (projects_to_save, projects_to_delete) = history.project_changes.iter().fold(
            (Vec::new(), Vec::new()),
            |mut changes, (before, after)| {
                let state = if use_after { after } else { before };
                match state {
                    Some(project) => changes.0.push(project.clone()),
                    None => changes.1.push(
                        before
                            .as_ref()
                            .or(after.as_ref())
                            .map(|project| project.id)
                            .expect("project history entry must have a project"),
                    ),
                }
                changes
            },
        );
        let (tags_to_save, tags_to_delete) = history.tag_changes.iter().fold(
            (Vec::new(), Vec::new()),
            |mut changes, (before, after)| {
                let state = if use_after { after } else { before };
                match state {
                    Some(tag) => changes.0.push(tag.clone()),
                    None => changes.1.push(
                        before
                            .as_ref()
                            .or(after.as_ref())
                            .map(|tag| tag.id)
                            .expect("tag history entry must have a tag"),
                    ),
                }
                changes
            },
        );
        self.worker.apply_history_state(
            (!history.task_changes.is_empty()).then(|| self.tasks.clone()),
            projects_to_save,
            projects_to_delete,
            tags_to_save,
            tags_to_delete,
        )
    }

    fn undo(&mut self, cx: &mut Context<Self>) {
        let Some(history) = self.undo_stack.pop() else {
            return;
        };
        self.apply_history_state(&history, false);
        if let Err(error) = self.persist_history_state(&history, false) {
            self.apply_history_state(&history, true);
            self.undo_stack.push(history);
            self.set_error(error);
        } else {
            self.redo_stack.push(history);
            self.status_message = "変更を取り消しました".to_owned();
        }
        cx.notify();
    }

    fn redo(&mut self, cx: &mut Context<Self>) {
        let Some(history) = self.redo_stack.pop() else {
            return;
        };
        self.apply_history_state(&history, true);
        if let Err(error) = self.persist_history_state(&history, true) {
            self.apply_history_state(&history, false);
            self.redo_stack.push(history);
            self.set_error(error);
        } else {
            self.undo_stack.push(history);
            self.status_message = "変更をやり直しました".to_owned();
        }
        cx.notify();
    }

    fn open_new_task_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.dismiss_due_popover(cx);
        self.due_input_error = None;
        if self.selected_task.is_some() {
            self.flush_pending_edits(cx);
        }
        self.selected_task = None;
        self.new_task_draft = Some(NewTaskDraft::default());
        self.title_input
            .update(cx, |state, cx| state.set_value("", window, cx));
        self.memo_input
            .update(cx, |state, cx| state.set_value("", window, cx));
        self.due_input
            .update(cx, |state, cx| state.set_value("", window, cx));
        self.due_calendar.update(cx, |state, cx| {
            state.set_date(PickerDate::Single(None), window, cx);
        });
        self.progress_input
            .update(cx, |state, cx| state.set_value("0", window, cx));
        self.error_message = None;
        self.title_input.read(cx).focus_handle(cx).focus(window);
        cx.notify();
    }

    fn close_task_form(&mut self, cx: &mut Context<Self>) {
        self.dismiss_due_popover(cx);
        if self.selected_task.is_some() {
            self.flush_pending_edits(cx);
        }
        self.selected_task = None;
        self.new_task_draft = None;
        cx.notify();
    }

    fn create_task(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(draft) = self.new_task_draft.clone() else {
            return false;
        };
        let now = OffsetDateTime::now_utc();
        let title = self.title_input.read(cx).value().to_string();
        let due = match parse_due(self.due_input.read(cx).value().as_str()) {
            Ok(due) => {
                self.due_input_error = None;
                due
            }
            Err(message) => {
                self.due_input_error = Some(message.clone());
                self.error_message = Some(message);
                cx.notify();
                return false;
            }
        };
        let progress = match self.progress_input.read(cx).value().trim().parse::<u8>() {
            Ok(progress) if progress <= 100 => progress,
            _ => {
                self.error_message = Some("進捗は0〜100の整数で入力してください".to_owned());
                cx.notify();
                return false;
            }
        };
        match Task::new(title, now) {
            Ok(mut task) => {
                task.memo = self.memo_input.read(cx).value().to_string();
                task.set_status(draft.status, now);
                task.priority = draft.priority;
                let _ = task.set_progress(progress);
                task.due = due;
                task.sort_order = self
                    .tasks
                    .iter()
                    .map(|task| task.sort_order)
                    .max()
                    .unwrap_or_default()
                    + 1024;
                if let Err(error) = self.worker.save_task(task.clone()) {
                    self.set_error(error);
                    cx.notify();
                    return false;
                }
                self.push_task_history(vec![(None, Some(task.clone()))]);
                self.tasks.push(task);
                self.new_task_draft = None;
                self.status_message = "新しいタスクを保存しました".to_owned();
                self.error_message = None;
                cx.notify();
                true
            }
            Err(error) => {
                self.error_message = Some(error.to_string());
                cx.notify();
                false
            }
        }
    }

    fn set_new_task_status(
        &mut self,
        status: TaskStatus,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(draft) = self.new_task_draft.as_mut() else {
            return;
        };
        if status == TaskStatus::Done {
            draft.progress = 100;
        } else if draft.status == TaskStatus::Done {
            draft.progress = 0;
        }
        draft.status = status;
        let progress = draft.progress.to_string();
        self.progress_input
            .update(cx, |state, cx| state.set_value(progress, window, cx));
        cx.notify();
    }

    fn set_new_task_priority(&mut self, priority: Priority, cx: &mut Context<Self>) {
        if let Some(draft) = self.new_task_draft.as_mut() {
            draft.priority = priority;
            cx.notify();
        }
    }

    fn set_new_task_progress(&mut self, progress: u8, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(draft) = self.new_task_draft.as_mut() {
            draft.progress = progress;
            self.progress_input.update(cx, |state, cx| {
                state.set_value(progress.to_string(), window, cx);
            });
            cx.notify();
        }
    }

    fn duplicate_task(&mut self, id: TaskId, cx: &mut Context<Self>) {
        let Some(source) = self.tasks.iter().find(|task| task.id == id).cloned() else {
            return;
        };
        let now = OffsetDateTime::now_utc();
        let Ok(mut copy) = Task::new(format!("{} のコピー", source.title), now) else {
            return;
        };
        copy.memo = source.memo;
        copy.status = source.status;
        copy.priority = source.priority;
        copy.progress = source.progress;
        copy.due = source.due;
        copy.project_id = source.project_id;
        copy.tag_ids = source.tag_ids;
        copy.sort_order = self
            .tasks
            .iter()
            .map(|task| task.sort_order)
            .max()
            .unwrap_or_default()
            + 1024;
        if let Err(error) = self.worker.save_task(copy.clone()) {
            self.set_error(error);
            return;
        }
        self.push_task_history(vec![(None, Some(copy.clone()))]);
        self.selected_task = Some(copy.id);
        self.tasks.push(copy);
        self.status_message = "タスクを複製しました".to_owned();
        cx.notify();
    }

    fn swap_task_order(&mut self, dragged: TaskId, target: TaskId, cx: &mut Context<Self>) {
        if self.view_kind == ViewKind::List {
            let tasks = self.visible_tasks(cx);
            let Some(left) = tasks.iter().find(|task| task.id == dragged) else {
                return;
            };
            let Some(right) = tasks.iter().find(|task| task.id == target) else {
                return;
            };
            if !self.can_reorder_list_pair(left, right) {
                return;
            }
        }
        self.persist_task_order_swap(dragged, target, "ドラッグで表示順を変更しました", cx);
    }

    fn persist_task_order_swap(
        &mut self,
        dragged: TaskId,
        target: TaskId,
        message: &str,
        cx: &mut Context<Self>,
    ) {
        if dragged == target {
            return;
        }
        let Some(left) = self.tasks.iter().position(|task| task.id == dragged) else {
            return;
        };
        let Some(right) = self.tasks.iter().position(|task| task.id == target) else {
            return;
        };
        let before_left = self.tasks[left].clone();
        let before_right = self.tasks[right].clone();
        let order = self.tasks[left].sort_order;
        self.tasks[left].sort_order = self.tasks[right].sort_order;
        self.tasks[right].sort_order = order;
        let changed = vec![self.tasks[left].clone(), self.tasks[right].clone()];
        if let Err(error) = self.worker.save_tasks(changed) {
            self.tasks[left] = before_left;
            self.tasks[right] = before_right;
            self.set_error(error);
        } else {
            self.push_task_history(vec![
                (Some(before_left), Some(self.tasks[left].clone())),
                (Some(before_right), Some(self.tasks[right].clone())),
            ]);
            self.status_message = message.to_owned();
        }
        cx.notify();
    }

    fn update_active_saved_view(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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

    fn save_current_view(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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

    fn delete_saved_view(&mut self, id: SavedViewId, cx: &mut Context<Self>) {
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

    fn set_filter_due_boundary(&mut self, from: bool, value: &str, cx: &mut Context<Self>) {
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

    fn toggle_task_selection(&mut self, id: TaskId, selected: bool, cx: &mut Context<Self>) {
        if selected {
            self.selected_tasks.insert(id);
        } else {
            self.selected_tasks.remove(&id);
        }
        cx.notify();
    }

    fn bulk_update(&mut self, cx: &mut Context<Self>, update: impl Fn(&mut Task, OffsetDateTime)) {
        let now = OffsetDateTime::now_utc();
        let mut changed = Vec::new();
        let mut history = Vec::new();
        for task in &mut self.tasks {
            if self.selected_tasks.contains(&task.id) {
                let before = task.clone();
                update(task, now);
                if before == *task {
                    continue;
                }
                history.push((Some(before), Some(task.clone())));
                changed.push(task.clone());
            }
        }
        if changed.is_empty() {
            return;
        }
        if let Err(error) = self.worker.save_tasks(changed) {
            for (before, _) in &history {
                if let Some(before) = before
                    && let Some(task) = self.tasks.iter_mut().find(|task| task.id == before.id)
                {
                    *task = before.clone();
                }
            }
            self.set_error(error);
        } else {
            self.push_task_history(history);
            self.status_message = format!("{}件を一括更新しました", self.selected_tasks.len());
        }
        cx.notify();
    }

    fn bulk_status(&mut self, status: TaskStatus, cx: &mut Context<Self>) {
        self.bulk_update(cx, |task, now| task.set_status(status, now));
    }

    fn bulk_priority(&mut self, priority: Priority, cx: &mut Context<Self>) {
        self.bulk_update(cx, |task, now| {
            task.priority = priority;
            task.touch(now);
        });
    }

    fn bulk_due(&mut self, cx: &mut Context<Self>) {
        let value = self.bulk_due_input.read(cx).value().to_string();
        match parse_due(&value) {
            Ok(due) => {
                self.bulk_update(cx, |task, now| {
                    task.due = due.clone();
                    task.touch(now);
                });
                self.error_message = None;
            }
            Err(message) => {
                self.error_message = Some(message);
                cx.notify();
            }
        }
    }

    fn bulk_trash(&mut self, cx: &mut Context<Self>) {
        self.bulk_update(cx, |task, now| task.move_to_trash(now));
        self.selected_tasks.clear();
    }

    fn purge_expired_trash_from_ui(&mut self, cx: &mut Context<Self>) {
        let now = OffsetDateTime::now_utc();
        if let Err(error) = self.worker.purge_expired_trash(now) {
            self.set_error(error);
            cx.notify();
            return;
        }
        let cutoff = now - time::Duration::days(30);
        let expired = self
            .tasks
            .iter()
            .filter(|task| task.deleted_at.is_some_and(|deleted| deleted <= cutoff))
            .map(|task| task.id)
            .collect::<HashSet<_>>();
        if expired.is_empty() {
            return;
        }
        self.tasks.retain(|task| !expired.contains(&task.id));
        self.selected_tasks.retain(|id| !expired.contains(id));
        if self.selected_task.is_some_and(|id| expired.contains(&id)) {
            self.selected_task = None;
        }
        if self
            .pending_title
            .as_ref()
            .is_some_and(|(id, _)| expired.contains(id))
        {
            self.pending_title = None;
            self.title_revision = self.title_revision.wrapping_add(1);
        }
        if self
            .pending_memo
            .as_ref()
            .is_some_and(|(id, _)| expired.contains(id))
        {
            self.pending_memo = None;
            self.memo_revision = self.memo_revision.wrapping_add(1);
        }
        self.undo_stack.clear();
        self.redo_stack.clear();
        self.status_message = format!("期限切れのゴミ箱タスクを{}件削除しました", expired.len());
        cx.notify();
    }

    fn create_manual_backup(&mut self, cx: &mut Context<Self>) {
        if let Err(message) = self.persist_pending_edits() {
            self.set_pending_edit_error(message);
            cx.notify();
            return;
        }
        let destination = self
            .paths
            .backups
            .join(format!("hodoq-manual-{}.sqlite3", unix_millis()));
        match self
            .worker
            .create_backup(destination.clone())
            .and_then(|_| self.worker.flush())
        {
            Ok(()) => {
                if let Some(error) = self.worker.take_error() {
                    self.error_message = Some(error);
                    self.status_message = "バックアップ失敗".to_owned();
                } else {
                    self.status_message = format!("バックアップを作成: {}", destination.display());
                    self.error_message = None;
                }
            }
            Err(error) => self.set_error(error),
        }
        cx.notify();
    }

    fn export_csv(&mut self, current_filter: bool, cx: &mut Context<Self>) {
        if let Err(message) = self.persist_pending_edits() {
            self.set_pending_edit_error(message);
            cx.notify();
            return;
        }
        let destination = self
            .paths
            .exports
            .join(format!("hodoq-{}.csv", unix_millis()));
        let tasks = if current_filter {
            self.visible_tasks(cx)
        } else {
            self.tasks.clone()
        };
        let result = self
            .worker
            .export_task_csv(destination.clone(), tasks, self.csv_with_bom)
            .and_then(|_| self.worker.flush());
        match result {
            Ok(()) => {
                if let Some(error) = self.worker.take_error() {
                    self.error_message = Some(error);
                    self.status_message = "CSV出力失敗".to_owned();
                } else {
                    self.status_message = format!("CSVを出力: {}", destination.display());
                    self.error_message = None;
                }
            }
            Err(error) => self.set_error(error),
        }
        cx.notify();
    }

    fn export_json(&mut self, cx: &mut Context<Self>) {
        if let Err(message) = self.persist_pending_edits() {
            self.set_pending_edit_error(message);
            cx.notify();
            return;
        }
        let destination = self
            .paths
            .exports
            .join(format!("hodoq-{}.json", unix_millis()));
        match self
            .worker
            .export_json(destination.clone())
            .and_then(|_| self.worker.flush())
        {
            Ok(()) => {
                if let Some(error) = self.worker.take_error() {
                    self.error_message = Some(error);
                    self.status_message = "JSON出力失敗".to_owned();
                } else {
                    self.status_message = format!("JSONを出力: {}", destination.display());
                    self.error_message = None;
                }
            }
            Err(error) => self.set_error(error),
        }
        cx.notify();
    }

    fn request_restore_path(&mut self, value: &str, cx: &mut Context<Self>) {
        let path = PathBuf::from(value.trim());
        if value.trim().is_empty() || !path.is_file() {
            self.error_message = Some("復元元のSQLiteファイルが見つかりません".to_owned());
            cx.notify();
            return;
        }
        self.error_message = None;
        self.pending_confirmation = Some(PendingConfirmation::Restore(path));
        cx.notify();
    }

    fn confirm_pending(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.pending_confirmation.take() {
            Some(PendingConfirmation::EmptyTrash) => {
                if let Err(message) = self.persist_pending_edits() {
                    self.set_pending_edit_error(message);
                    self.pending_confirmation = Some(PendingConfirmation::EmptyTrash);
                    cx.notify();
                    return;
                }
                if let Err(error) = self.worker.empty_trash().and_then(|_| self.worker.flush()) {
                    self.set_error(error);
                } else if let Some(error) = self.worker.take_error() {
                    self.error_message = Some(error);
                } else {
                    self.tasks.retain(|task| task.deleted_at.is_none());
                    self.selected_task = None;
                    self.selected_tasks.clear();
                    self.undo_stack.clear();
                    self.redo_stack.clear();
                    self.status_message = "ゴミ箱を空にしました".to_owned();
                }
            }
            Some(PendingConfirmation::Restore(source)) => {
                if let Err(message) = self.persist_pending_edits() {
                    self.set_pending_edit_error(message);
                    self.pending_confirmation = Some(PendingConfirmation::Restore(source));
                    cx.notify();
                    return;
                }
                let safety = self
                    .paths
                    .backups
                    .join(format!("hodoq-before-restore-{}.sqlite3", unix_millis()));
                match self.worker.restore_backup(source, safety) {
                    Ok(snapshot) => {
                        self.title_revision = self.title_revision.wrapping_add(1);
                        self.memo_revision = self.memo_revision.wrapping_add(1);
                        self.pending_title = None;
                        self.pending_memo = None;
                        self.tasks = snapshot.tasks;
                        self.projects = snapshot.projects;
                        self.tags = snapshot.tags;
                        self.saved_views = snapshot.saved_views;
                        self.selected_task = None;
                        self.selected_tasks.clear();
                        self.undo_stack.clear();
                        self.redo_stack.clear();
                        self.active_view = SmartView::All;
                        self.status_message = "バックアップから復元しました".to_owned();
                        self.restore_path_input
                            .update(cx, |state, cx| state.set_value("", window, cx));
                        self.sync_detail_inputs(window, cx);
                    }
                    Err(error) => self.set_error(error),
                }
            }
            Some(PendingConfirmation::CloseSaveFailed) => {
                self.retry_close(window, cx);
                return;
            }
            None => {}
        }
        cx.notify();
    }

    fn select_task(&mut self, id: TaskId, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_task != Some(id) {
            self.flush_pending_edits(cx);
        }
        self.selected_task = Some(id);
        self.new_task_draft = None;
        self.selection_anchor = Some(id);
        self.sync_detail_inputs(window, cx);
        cx.notify();
    }

    fn flush_pending_edits(&mut self, cx: &mut Context<Self>) {
        self.title_revision = self.title_revision.wrapping_add(1);
        self.memo_revision = self.memo_revision.wrapping_add(1);
        if let Err(message) = self.persist_pending_edits() {
            self.set_pending_edit_error(message);
        }
        cx.notify();
    }

    fn persist_pending_edits(&mut self) -> Result<(), String> {
        if self.pending_title.is_none() && self.pending_memo.is_none() {
            return Ok(());
        }
        let mut tasks = self.tasks.clone();
        apply_pending_edits(
            &mut tasks,
            self.pending_title.as_ref(),
            self.pending_memo.as_ref(),
            OffsetDateTime::now_utc(),
        )?;
        let history = self
            .tasks
            .iter()
            .zip(&tasks)
            .filter(|(before, after)| before != after)
            .map(|(before, after)| (Some(before.clone()), Some(after.clone())))
            .collect::<Vec<_>>();
        if !history.is_empty() {
            self.worker
                .save_tasks(
                    history
                        .iter()
                        .filter_map(|(_, after)| after.clone())
                        .collect(),
                )
                .map_err(|error| error.to_string())?;
        }
        self.tasks = tasks;
        self.pending_title = None;
        self.pending_memo = None;
        self.push_task_history(history);
        self.error_message = None;
        self.status_message = "保存済み".to_owned();
        Ok(())
    }

    fn set_pending_edit_error(&mut self, message: String) {
        while self.worker.take_error().is_some() {}
        self.error_message = Some(message);
        self.status_message = if self.worker.is_read_only() {
            "読み取り専用".to_owned()
        } else {
            "保存失敗".to_owned()
        };
    }

    pub(super) fn should_close(&mut self, cx: &mut Context<Self>) -> bool {
        if self.allow_close || self.close_save_completed {
            return true;
        }
        match self.persist_before_close() {
            Ok(()) => true,
            Err(message) => {
                while self.worker.take_error().is_some() {}
                self.error_message = Some(format!("終了前の保存に失敗しました: {message}"));
                self.status_message = "保存失敗 — 終了を保留中".to_owned();
                self.pending_confirmation = Some(PendingConfirmation::CloseSaveFailed);
                cx.notify();
                false
            }
        }
    }

    fn persist_before_close(&mut self) -> Result<(), String> {
        self.persist_pending_edits()?;
        self.worker.flush().map_err(|error| error.to_string())?;
        self.update_persisted_settings_fields();
        self.settings
            .save(&self.paths.settings)
            .map_err(|error| error.to_string())?;
        self.close_save_completed = true;
        self.status_message = "保存済み".to_owned();
        Ok(())
    }

    fn update_persisted_settings_fields(&mut self) {
        self.settings.view_kind = self.view_kind;
        self.settings.active_view = smart_view_setting(self.active_view);
        self.settings.sort = self.sort.clone();
        self.settings.group_by = self.group_by;
    }

    fn retry_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.pending_confirmation = None;
        if self.should_close(cx) {
            self.allow_close = true;
            window.remove_window();
        }
    }

    fn discard_unsaved_and_close(&mut self, window: &mut Window) {
        self.pending_title = None;
        self.pending_memo = None;
        self.pending_confirmation = None;
        self.discard_unsaved_on_close = true;
        self.allow_close = true;
        window.remove_window();
    }

    fn handle_task_click(
        &mut self,
        id: TaskId,
        shift: bool,
        secondary: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if shift {
            let tasks = self.visible_tasks(cx);
            let anchor = self.selection_anchor.or(self.selected_task).unwrap_or(id);
            let Some(anchor_index) = tasks.iter().position(|task| task.id == anchor) else {
                self.select_task(id, window, cx);
                return;
            };
            let Some(clicked_index) = tasks.iter().position(|task| task.id == id) else {
                return;
            };
            let (start, end) = if anchor_index <= clicked_index {
                (anchor_index, clicked_index)
            } else {
                (clicked_index, anchor_index)
            };
            self.selection_mode = true;
            if !secondary {
                self.selected_tasks.clear();
            }
            self.selected_tasks
                .extend(tasks[start..=end].iter().map(|task| task.id));
            self.selected_task = Some(id);
            self.sync_detail_inputs(window, cx);
            cx.notify();
        } else if secondary {
            self.selection_mode = true;
            if !self.selected_tasks.insert(id) {
                self.selected_tasks.remove(&id);
            }
            self.selected_task = Some(id);
            self.selection_anchor = Some(id);
            self.sync_detail_inputs(window, cx);
            cx.notify();
        } else {
            self.select_task(id, window, cx);
        }
    }

    fn sync_management_inputs(
        &mut self,
        view: SmartView,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let name = match view {
            SmartView::Saved(id) => self
                .saved_views
                .iter()
                .find(|view| view.id == id)
                .map(|view| view.name.clone())
                .unwrap_or_default(),
            _ => String::new(),
        };
        self.view_name_input.update(cx, |state, cx| {
            state.set_value(name, window, cx);
        });
    }

    fn sync_detail_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.dismiss_due_popover(cx);
        self.due_input_error = None;
        let Some(task) = self.selected_task().cloned() else {
            return;
        };
        let picker_date = picker_date_from_due(&task.due);
        self.title_input.update(cx, |state, cx| {
            state.set_value(task.title, window, cx);
        });
        self.memo_input.update(cx, |state, cx| {
            state.set_value(task.memo, window, cx);
        });
        self.due_input.update(cx, |state, cx| {
            state.set_value(format_due_input(&task.due), window, cx);
        });
        self.due_calendar.update(cx, |state, cx| {
            state.set_date(picker_date, window, cx);
        });
        self.progress_input.update(cx, |state, cx| {
            state.set_value(task.progress.to_string(), window, cx);
        });
    }

    fn schedule_title_save(&mut self, title: String, cx: &mut Context<Self>) {
        let Some(id) = self.selected_task else {
            return;
        };
        if title.trim().is_empty() || title.chars().count() > 500 {
            self.error_message = Some("タイトルは1〜500文字で入力してください".to_owned());
            cx.notify();
            return;
        }
        self.title_revision = self.title_revision.wrapping_add(1);
        let revision = self.title_revision;
        self.pending_title = Some((id, title));
        self.status_message = "編集中…".to_owned();
        cx.spawn(async move |this, cx| {
            Timer::after(StdDuration::from_millis(400)).await;
            if let Some(this) = this.upgrade() {
                this.update(cx, |this, cx| {
                    if this.title_revision != revision {
                        return;
                    }
                    if this.pending_title.is_none() {
                        return;
                    }
                    if let Err(message) = this.persist_pending_edits() {
                        this.set_pending_edit_error(message);
                    }
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
        cx.notify();
    }

    fn schedule_memo_save(&mut self, memo: String, cx: &mut Context<Self>) {
        let Some(id) = self.selected_task else {
            return;
        };
        self.memo_revision = self.memo_revision.wrapping_add(1);
        let revision = self.memo_revision;
        self.pending_memo = Some((id, memo));
        self.status_message = "編集中…".to_owned();
        cx.spawn(async move |this, cx| {
            Timer::after(StdDuration::from_millis(400)).await;
            if let Some(this) = this.upgrade() {
                this.update(cx, |this, cx| {
                    if this.memo_revision != revision {
                        return;
                    }
                    if this.pending_memo.is_none() {
                        return;
                    }
                    if let Err(message) = this.persist_pending_edits() {
                        this.set_pending_edit_error(message);
                    }
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
        cx.notify();
    }

    fn save_selected_task_form(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(id) = self.selected_task else {
            return false;
        };
        let Some(before) = self.tasks.iter().find(|task| task.id == id).cloned() else {
            return false;
        };
        let mut after = before.clone();
        if let Err(error) = after.set_title(self.title_input.read(cx).value().to_string()) {
            self.error_message = Some(error.to_string());
            cx.notify();
            return false;
        }
        after.memo = self.memo_input.read(cx).value().to_string();
        after.due = match parse_due(self.due_input.read(cx).value().as_str()) {
            Ok(due) => {
                self.due_input_error = None;
                due
            }
            Err(message) => {
                self.due_input_error = Some(message.clone());
                self.error_message = Some(message);
                cx.notify();
                return false;
            }
        };
        let progress = match self.progress_input.read(cx).value().trim().parse::<u8>() {
            Ok(progress) if progress <= 100 => progress,
            _ => {
                self.error_message = Some("進捗は0〜100の整数で入力してください".to_owned());
                cx.notify();
                return false;
            }
        };
        let _ = after.set_progress(progress);

        if after != before {
            after.touch(OffsetDateTime::now_utc());
            if let Err(error) = self.worker.save_task(after.clone()) {
                self.set_error(error);
                cx.notify();
                return false;
            }
            if let Some(task) = self.tasks.iter_mut().find(|task| task.id == id) {
                *task = after.clone();
            }
            self.push_task_history(vec![(Some(before), Some(after))]);
        }

        self.title_revision = self.title_revision.wrapping_add(1);
        self.memo_revision = self.memo_revision.wrapping_add(1);
        self.pending_title = None;
        self.pending_memo = None;
        self.error_message = None;
        self.status_message = "タスクの変更を保存しました".to_owned();
        cx.notify();
        true
    }

    fn save_and_close_selected_task(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.save_selected_task_form(cx) {
            return false;
        }
        self.selected_task = None;
        self.new_task_draft = None;
        cx.notify();
        true
    }

    fn set_task_status(&mut self, id: TaskId, status: TaskStatus, cx: &mut Context<Self>) {
        self.update_task(id, cx, |task, now| task.set_status(status, now));
    }

    fn set_task_priority(&mut self, id: TaskId, priority: Priority, cx: &mut Context<Self>) {
        self.update_task(id, cx, |task, now| {
            task.priority = priority;
            task.touch(now);
        });
    }

    fn set_task_progress(&mut self, id: TaskId, progress: u8, cx: &mut Context<Self>) {
        self.update_task(id, cx, |task, now| {
            let _ = task.set_progress(progress);
            task.touch(now);
        });
    }

    fn move_to_trash(&mut self, id: TaskId, cx: &mut Context<Self>) {
        let now = OffsetDateTime::now_utc();
        let mut history = None;
        if let Some(task) = self.tasks.iter_mut().find(|task| task.id == id) {
            let before = task.clone();
            task.move_to_trash(now);
            history = Some((Some(before), Some(task.clone())));
        }
        match self.worker.move_task_to_trash(id, now) {
            Err(error) => {
                if let Some((Some(before), _)) = &history
                    && let Some(task) = self.tasks.iter_mut().find(|task| task.id == id)
                {
                    *task = before.clone();
                }
                self.set_error(error);
            }
            Ok(()) => {
                if self.selected_task == Some(id) {
                    self.selected_task = None;
                }
                if let Some(change) = history {
                    self.push_task_history(vec![change]);
                }
                self.status_message = "ゴミ箱へ移動しました".to_owned();
            }
        }
        cx.notify();
    }

    fn restore_task(&mut self, id: TaskId, cx: &mut Context<Self>) {
        let now = OffsetDateTime::now_utc();
        let mut history = None;
        if let Some(task) = self.tasks.iter_mut().find(|task| task.id == id) {
            let before = task.clone();
            task.restore(now);
            history = Some((Some(before), Some(task.clone())));
        }
        match self.worker.restore_task(id, now) {
            Err(error) => {
                if let Some((Some(before), _)) = &history
                    && let Some(task) = self.tasks.iter_mut().find(|task| task.id == id)
                {
                    *task = before.clone();
                }
                self.set_error(error);
            }
            Ok(()) => {
                if let Some(change) = history {
                    self.push_task_history(vec![change]);
                }
                self.status_message = "タスクを復元しました".to_owned();
            }
        }
        cx.notify();
    }

    fn update_selected_task(
        &mut self,
        cx: &mut Context<Self>,
        update: impl FnOnce(&mut Task, OffsetDateTime),
    ) {
        if let Some(id) = self.selected_task {
            self.update_task(id, cx, update);
        }
    }

    fn update_task(
        &mut self,
        id: TaskId,
        cx: &mut Context<Self>,
        update: impl FnOnce(&mut Task, OffsetDateTime),
    ) {
        let now = OffsetDateTime::now_utc();
        let Some(task) = self.tasks.iter_mut().find(|task| task.id == id) else {
            return;
        };
        let before = task.clone();
        update(task, now);
        if before == *task {
            return;
        }
        let task = task.clone();
        if let Err(error) = self.worker.save_task(task) {
            if let Some(task) = self.tasks.iter_mut().find(|task| task.id == id) {
                *task = before;
            }
            self.set_error(error);
        } else {
            let after = self
                .tasks
                .iter()
                .find(|task| task.id == id)
                .cloned()
                .expect("updated task must exist");
            self.push_task_history(vec![(Some(before), Some(after))]);
            self.status_message = "保存中…".to_owned();
        }
        cx.notify();
    }

    fn set_error(&mut self, error: RepositoryError) {
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

    fn visible_tasks(&self, cx: &Context<Self>) -> Vec<Task> {
        let query = self.search_input.read(cx).value().to_lowercase();
        let now = OffsetDateTime::now_utc();
        let offset = UtcOffset::current_local_offset().unwrap_or(UtcOffset::UTC);
        let today = now.to_offset(offset).date();
        let saved_query = match self.active_view {
            SmartView::Saved(id) => self
                .saved_views
                .iter()
                .find(|view| view.id == id)
                .map(|view| TaskQuery::new(&view.filter, now, offset)),
            _ => None,
        };
        let mut tasks = self
            .tasks
            .iter()
            .filter(|task| {
                if !query.is_empty()
                    && !task.title.to_lowercase().contains(query.as_str())
                    && !task.memo.to_lowercase().contains(query.as_str())
                {
                    return false;
                }
                if !self.filter_statuses.is_empty() && !self.filter_statuses.contains(&task.status)
                {
                    return false;
                }
                if !self.filter_priorities.is_empty()
                    && !self.filter_priorities.contains(&task.priority)
                {
                    return false;
                }
                let project_filter_active =
                    !self.filter_projects.is_empty() || self.filter_unassigned_project;
                let project_matches = task
                    .project_id
                    .map_or(self.filter_unassigned_project, |id| {
                        self.filter_projects.contains(&id)
                    });
                if project_filter_active && !project_matches {
                    return false;
                }
                if !self.filter_tags.is_empty() {
                    let matches = if self.filter_match_all_tags {
                        self.filter_tags.iter().all(|id| task.tag_ids.contains(id))
                    } else {
                        self.filter_tags.iter().any(|id| task.tag_ids.contains(id))
                    };
                    if !matches {
                        return false;
                    }
                }
                let due_matches = match self.filter_due {
                    DueScope::Any => true,
                    DueScope::Undated => matches!(task.due, Due::None),
                    DueScope::Today => due_is_today(&task.due, today, offset),
                    DueScope::Upcoming => due_is_upcoming(&task.due, today, offset),
                    DueScope::Overdue => {
                        task.status != TaskStatus::Done && task.due.is_overdue(now, today)
                    }
                };
                if !due_matches {
                    return false;
                }
                let task_due_date = due_date(&task.due);
                if self
                    .filter_due_from
                    .is_some_and(|from| task_due_date.is_none_or(|date| date < from))
                    || self
                        .filter_due_to
                        .is_some_and(|to| task_due_date.is_none_or(|date| date > to))
                {
                    return false;
                }
                match self.active_view {
                    SmartView::Trash => task.deleted_at.is_some(),
                    SmartView::Saved(_) => saved_query
                        .as_ref()
                        .is_some_and(|query| query.matches(task)),
                    _ if task.deleted_at.is_some() => false,
                    SmartView::Archived => task.status == TaskStatus::Archived,
                    _ if task.status == TaskStatus::Archived => false,
                    SmartView::Today => due_is_today(&task.due, today, offset),
                    SmartView::Upcoming => due_is_upcoming(&task.due, today, offset),
                    SmartView::Overdue => {
                        task.due.is_overdue(now, today) && task.status != TaskStatus::Done
                    }
                    SmartView::Undated => matches!(task.due, Due::None),
                    SmartView::All => true,
                    SmartView::Doing => task.status == TaskStatus::Doing,
                    SmartView::Blocked => task.status == TaskStatus::Blocked,
                    SmartView::Done => task.status == TaskStatus::Done,
                }
            })
            .cloned()
            .collect::<Vec<_>>();
        tasks.sort_by(|left, right| compare_tasks(left, right, &self.sort, offset));
        if self.view_kind == ViewKind::List {
            self.order_list_tasks(&mut tasks);
        }
        tasks
    }

    fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity();
        let view_kind = self.view_kind;
        div()
            .flex()
            .flex_col()
            .border_b_1()
            .border_color(theme::BORDER)
            .bg(theme::SURFACE)
            .child(
                div()
                    .flex()
                    .items_center()
                    .flex_wrap()
                    .gap_3()
                    .min_h(px(64.0))
                    .py_2()
                    .px_4()
                    .child(
                        div()
                            .text_size(px(22.0))
                            .font_weight(FontWeight::BOLD)
                            .text_color(theme::TEXT)
                            .w(px(130.0))
                            .child("HodoQ"),
                    )
                    .child(
                        Button::new("open-new-task")
                            .primary()
                            .label("新規タスク")
                            .selected(self.new_task_draft.is_some())
                            .on_click(move |_, window, cx| {
                                entity.update(cx, |this, cx| {
                                    this.open_new_task_form(window, cx);
                                });
                            }),
                    )
                    .child(div().flex_1())
                    .child(Input::new(&self.search_input).cleanable(true).w(px(240.0)))
                    .children(
                        [
                            (ViewKind::List, "リスト"),
                            (ViewKind::Board, "ボード"),
                            (ViewKind::Calendar, "カレンダー"),
                        ]
                        .into_iter()
                        .map(|(kind, label)| {
                            let entity = cx.entity();
                            Button::new(SharedString::from(format!("view-{label}")))
                                .small()
                                .label(label)
                                .selected(view_kind == kind)
                                .on_click(move |_, _, cx| {
                                    entity.update(cx, |this, cx| {
                                        this.view_kind = kind;
                                        this.settings.view_kind = kind;
                                        cx.notify();
                                    });
                                })
                        }),
                    )
                    .child({
                        let entity = cx.entity();
                        Button::new("toggle-more-menu")
                            .small()
                            .label(if self.show_more_menu {
                                "その他 ▲"
                            } else {
                                "その他 ▼"
                            })
                            .selected(self.show_more_menu)
                            .on_click(move |_, _, cx| {
                                entity.update(cx, |this, cx| {
                                    this.show_more_menu = !this.show_more_menu;
                                    cx.notify();
                                });
                            })
                    }),
            )
            .when(self.show_more_menu, |toolbar| {
                toolbar.child(self.render_more_menu(cx))
            })
            .when(self.show_filter_panel, |toolbar| {
                toolbar.child(self.render_filter_panel(cx))
            })
            .when(self.selection_mode, |toolbar| {
                toolbar.child(self.render_bulk_bar(cx))
            })
            .when(self.show_data_panel, |toolbar| {
                toolbar.child(self.render_data_panel(cx))
            })
            .when(self.show_command_palette, |toolbar| {
                toolbar.child(self.render_command_palette(cx))
            })
            .when_some(
                self.pending_confirmation.clone(),
                |toolbar, confirmation| toolbar.child(self.render_confirmation(confirmation, cx)),
            )
    }

    fn render_more_menu(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex()
            .items_center()
            .flex_wrap()
            .gap_2()
            .px_4()
            .pb_3()
            .child(self.small_action_button(
                "toggle-filters",
                "絞り込み・並び替え",
                cx,
                |this, _, cx| {
                    this.show_filter_panel = !this.show_filter_panel;
                    this.show_more_menu = false;
                    cx.notify();
                },
            ))
            .child(self.small_action_button(
                "toggle-selection",
                "複数選択",
                cx,
                |this, _, cx| {
                    this.selection_mode = !this.selection_mode;
                    if !this.selection_mode {
                        this.selected_tasks.clear();
                    }
                    this.show_more_menu = false;
                    cx.notify();
                },
            ))
            .child(
                self.small_action_button("toggle-data", "データ管理", cx, |this, _, cx| {
                    this.show_data_panel = !this.show_data_panel;
                    this.show_more_menu = false;
                    cx.notify();
                }),
            )
            .child(self.small_action_button(
                "toggle-command-palette",
                "操作を検索",
                cx,
                |this, window, cx| {
                    this.show_command_palette = !this.show_command_palette;
                    this.show_more_menu = false;
                    if this.show_command_palette {
                        this.command_input.read(cx).focus_handle(cx).focus(window);
                    }
                    cx.notify();
                },
            ))
            .child(self.small_action_button("undo", "元に戻す", cx, |this, _, cx| this.undo(cx)))
            .child(self.small_action_button("redo", "やり直す", cx, |this, _, cx| this.redo(cx)))
            .when(self.worker.is_read_only(), |menu| {
                menu.child(self.small_action_button(
                    "retry-database",
                    "DBを再試行",
                    cx,
                    |this, _, cx| this.retry_database(cx),
                ))
            })
            .into_any_element()
    }

    fn render_command_palette(&self, cx: &mut Context<Self>) -> AnyElement {
        let query = self.command_input.read(cx).value().to_lowercase();
        let commands = [
            (PaletteCommand::NewTask, "新しいタスク", "新規 追加 task"),
            (PaletteCommand::Search, "タスクを検索", "検索 find search"),
            (PaletteCommand::Today, "今日へ移動", "今日 today view"),
            (PaletteCommand::All, "すべてへ移動", "全部 all view"),
            (
                PaletteCommand::StatusTodo,
                "選択タスクを未着手へ",
                "状態 todo 未着手",
            ),
            (
                PaletteCommand::StatusDoing,
                "選択タスクを進行中へ",
                "状態 doing 進行",
            ),
            (
                PaletteCommand::StatusBlocked,
                "選択タスクを保留へ",
                "状態 blocked 保留",
            ),
            (
                PaletteCommand::StatusDone,
                "選択タスクを完了へ",
                "状態 done 完了",
            ),
            (
                PaletteCommand::StatusArchived,
                "選択タスクをアーカイブへ",
                "状態 archived アーカイブ",
            ),
            (
                PaletteCommand::PriorityNone,
                "選択タスクの優先度をなしへ",
                "優先度 none なし",
            ),
            (
                PaletteCommand::PriorityLow,
                "選択タスクを低優先度へ",
                "優先度 low 低",
            ),
            (
                PaletteCommand::PriorityMedium,
                "選択タスクを中優先度へ",
                "優先度 medium 中",
            ),
            (
                PaletteCommand::PriorityHigh,
                "選択タスクを高優先度へ",
                "優先度 high 高",
            ),
            (
                PaletteCommand::Backup,
                "手動バックアップを作成",
                "backup 保存 バックアップ",
            ),
        ];
        div()
            .flex()
            .flex_col()
            .gap_2()
            .px_4()
            .pb_3()
            .border_t_1()
            .border_color(theme::BORDER)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .pt_3()
                    .child(Input::new(&self.command_input).cleanable(true).w(px(420.0)))
                    .child(self.small_action_button(
                        "close-command-palette",
                        "閉じる",
                        cx,
                        |this, _, cx| {
                            this.show_command_palette = false;
                            cx.notify();
                        },
                    )),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .children(
                        commands
                            .into_iter()
                            .filter_map(|(command, label, keywords)| {
                                let haystack = format!("{label} {keywords}").to_lowercase();
                                if !query.is_empty() && !haystack.contains(query.trim()) {
                                    return None;
                                }
                                let entity = cx.entity();
                                Some(
                                    Button::new(SharedString::from(format!("palette-{command:?}")))
                                        .small()
                                        .label(label)
                                        .on_click(move |_, window, cx| {
                                            entity.update(cx, |this, cx| {
                                                this.execute_palette_command(command, window, cx);
                                            });
                                        }),
                                )
                            }),
                    ),
            )
            .into_any_element()
    }

    fn execute_palette_command(
        &mut self,
        command: PaletteCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match command {
            PaletteCommand::NewTask => self.open_new_task_form(window, cx),
            PaletteCommand::Search => self.search_input.read(cx).focus_handle(cx).focus(window),
            PaletteCommand::Today => self.active_view = SmartView::Today,
            PaletteCommand::All => self.active_view = SmartView::All,
            PaletteCommand::StatusTodo => self.set_selected_task_status(TaskStatus::Todo, cx),
            PaletteCommand::StatusDoing => self.set_selected_task_status(TaskStatus::Doing, cx),
            PaletteCommand::StatusBlocked => self.set_selected_task_status(TaskStatus::Blocked, cx),
            PaletteCommand::StatusDone => self.set_selected_task_status(TaskStatus::Done, cx),
            PaletteCommand::StatusArchived => {
                self.set_selected_task_status(TaskStatus::Archived, cx)
            }
            PaletteCommand::PriorityNone => self.set_selected_task_priority(Priority::None, cx),
            PaletteCommand::PriorityLow => self.set_selected_task_priority(Priority::Low, cx),
            PaletteCommand::PriorityMedium => self.set_selected_task_priority(Priority::Medium, cx),
            PaletteCommand::PriorityHigh => self.set_selected_task_priority(Priority::High, cx),
            PaletteCommand::Backup => self.create_manual_backup(cx),
        }
        self.show_command_palette = false;
        self.command_input
            .update(cx, |state, cx| state.set_value("", window, cx));
        cx.notify();
    }

    fn set_selected_task_status(&mut self, status: TaskStatus, cx: &mut Context<Self>) {
        if let Some(id) = self.selected_task {
            self.set_task_status(id, status, cx);
        }
    }

    fn set_selected_task_priority(&mut self, priority: Priority, cx: &mut Context<Self>) {
        if let Some(id) = self.selected_task {
            self.set_task_priority(id, priority, cx);
        }
    }

    fn retry_database(&mut self, cx: &mut Context<Self>) {
        match TaskApplication::start(&self.paths.database).and_then(|worker| {
            let snapshot = worker.load()?;
            if worker.is_read_only() {
                return Err(RepositoryError::ReadOnly);
            }
            Ok((worker, snapshot))
        }) {
            Ok((worker, snapshot)) => {
                self.worker = worker;
                self.tasks = snapshot.tasks;
                self.projects = snapshot.projects;
                self.tags = snapshot.tags;
                self.saved_views = snapshot.saved_views;
                if let Err(message) = self.persist_pending_edits() {
                    self.set_pending_edit_error(message);
                } else {
                    self.error_message = None;
                    self.status_message = "通常モードへ復帰しました".to_owned();
                }
            }
            Err(error) => {
                self.error_message = Some(format!("DBの再試行に失敗しました: {error}"));
                self.status_message = "読み取り専用".to_owned();
            }
        }
        cx.notify();
    }

    fn move_selection(&mut self, direction: i32, window: &mut Window, cx: &mut Context<Self>) {
        let tasks = self.visible_tasks(cx);
        if tasks.is_empty() {
            return;
        }
        let current = self
            .selected_task
            .and_then(|id| tasks.iter().position(|task| task.id == id));
        let index = match (current, direction.is_negative()) {
            (Some(index), true) => index.saturating_sub(1),
            (Some(index), false) => (index + 1).min(tasks.len() - 1),
            (None, true) => tasks.len() - 1,
            (None, false) => 0,
        };
        self.select_task(tasks[index].id, window, cx);
    }

    fn toggle_selected_done(&mut self, cx: &mut Context<Self>) {
        let Some(task) = self.selected_task().cloned() else {
            return;
        };
        self.set_task_status(
            task.id,
            if task.status == TaskStatus::Done {
                TaskStatus::Todo
            } else {
                TaskStatus::Done
            },
            cx,
        );
    }

    fn render_filter_panel(&self, cx: &mut Context<Self>) -> AnyElement {
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

    fn render_bulk_bar(&self, cx: &mut Context<Self>) -> AnyElement {
        let count = self.selected_tasks.len();
        div()
            .flex()
            .items_center()
            .flex_wrap()
            .gap_2()
            .px_4()
            .pb_3()
            .child(format!("{count}件を選択"))
            .child(self.small_action_button(
                "bulk-all",
                "表示中を全選択",
                cx,
                |this, _, cx| {
                    this.selected_tasks = this
                        .visible_tasks(cx)
                        .into_iter()
                        .map(|task| task.id)
                        .collect();
                    cx.notify();
                },
            ))
            .children(TaskStatus::ALL.into_iter().map(|status| {
                let entity = cx.entity();
                Button::new(SharedString::from(format!(
                    "bulk-status-{}",
                    status.as_str()
                )))
                .small()
                .label(format!("状態 {}", status.label()))
                .on_click(move |_, _, cx| {
                    entity.update(cx, |this, cx| this.bulk_status(status, cx));
                })
            }))
            .children(Priority::ALL.into_iter().map(|priority| {
                let entity = cx.entity();
                Button::new(SharedString::from(format!(
                    "bulk-priority-{}",
                    priority.as_str()
                )))
                .small()
                .label(format!("優先度 {}", priority.label()))
                .on_click(move |_, _, cx| {
                    entity.update(cx, |this, cx| this.bulk_priority(priority, cx));
                })
            }))
            .child(Input::new(&self.bulk_due_input).small().w(px(260.0)))
            .child(
                self.small_action_button("bulk-due", "納期を設定", cx, |this, _, cx| {
                    this.bulk_due(cx);
                }),
            )
            .child(
                self.small_action_button("bulk-due-clear", "納期なし", cx, |this, _, cx| {
                    this.bulk_update(cx, |task, now| {
                        task.due = Due::None;
                        task.touch(now);
                    });
                }),
            )
            .child({
                let entity = cx.entity();
                Button::new("bulk-trash")
                    .small()
                    .danger()
                    .label("まとめて削除")
                    .on_click(move |_, _, cx| {
                        entity.update(cx, |this, cx| this.bulk_trash(cx));
                    })
            })
            .into_any_element()
    }

    fn render_data_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let backups = list_backup_files(&self.paths.backups);
        div()
            .flex()
            .flex_col()
            .gap_2()
            .px_4()
            .pb_3()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(self.small_action_button(
                        "manual-backup",
                        "手動バックアップ",
                        cx,
                        |this, _, cx| {
                            this.create_manual_backup(cx);
                        },
                    ))
                    .child({
                        let entity = cx.entity();
                        Button::new("csv-bom-mode")
                            .small()
                            .label(if self.csv_with_bom {
                                "CSV: BOMあり"
                            } else {
                                "CSV: BOMなし"
                            })
                            .on_click(move |_, _, cx| {
                                entity.update(cx, |this, cx| {
                                    this.csv_with_bom = !this.csv_with_bom;
                                    cx.notify();
                                });
                            })
                    })
                    .child(self.small_action_button(
                        "export-current-csv",
                        "表示結果をCSV",
                        cx,
                        |this, _, cx| {
                            this.export_csv(true, cx);
                        },
                    ))
                    .child(self.small_action_button(
                        "export-all-csv",
                        "全タスクをCSV",
                        cx,
                        |this, _, cx| {
                            this.export_csv(false, cx);
                        },
                    ))
                    .child(self.small_action_button(
                        "export-json",
                        "全データをJSON",
                        cx,
                        |this, _, cx| {
                            this.export_json(cx);
                        },
                    ))
                    .child({
                        let entity = cx.entity();
                        Button::new("empty-trash")
                            .small()
                            .danger()
                            .label("ゴミ箱を空にする")
                            .on_click(move |_, _, cx| {
                                entity.update(cx, |this, cx| {
                                    this.pending_confirmation =
                                        Some(PendingConfirmation::EmptyTrash);
                                    cx.notify();
                                });
                            })
                    })
                    .child(
                        div()
                            .ml_3()
                            .text_size(px(12.0))
                            .text_color(theme::MUTED)
                            .child(format!("保存先: {}", self.paths.data_dir.display())),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().text_color(theme::MUTED).child("任意ファイルから復元"))
                    .child(Input::new(&self.restore_path_input).small().flex_1())
                    .child({
                        let entity = cx.entity();
                        Button::new("restore-from-path")
                            .small()
                            .label("復元内容を確認")
                            .on_click(move |_, _, cx| {
                                entity.update(cx, |this, cx| {
                                    let value =
                                        this.restore_path_input.read(cx).value().to_string();
                                    this.request_restore_path(&value, cx);
                                });
                            })
                    }),
            )
            .when(!backups.is_empty(), |panel| {
                panel.child(
                    div()
                        .flex()
                        .items_center()
                        .flex_wrap()
                        .gap_2()
                        .child(
                            div()
                                .text_color(theme::MUTED)
                                .child("復元可能なバックアップ"),
                        )
                        .children(backups.into_iter().map(|path| {
                            let entity = cx.entity();
                            let label = path
                                .file_name()
                                .and_then(|name| name.to_str())
                                .unwrap_or("backup.sqlite3")
                                .to_owned();
                            Button::new(SharedString::from(format!("restore-{}", path.display())))
                                .small()
                                .label(label)
                                .on_click(move |_, _, cx| {
                                    let path = path.clone();
                                    entity.update(cx, |this, cx| {
                                        this.pending_confirmation =
                                            Some(PendingConfirmation::Restore(path));
                                        cx.notify();
                                    });
                                })
                        })),
                )
            })
            .into_any_element()
    }

    fn render_confirmation(
        &self,
        confirmation: PendingConfirmation,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let close_save_failed = matches!(confirmation, PendingConfirmation::CloseSaveFailed);
        let message = match &confirmation {
            PendingConfirmation::EmptyTrash => {
                "ゴミ箱内のタスクを完全に削除します。この操作は取り消せません。".to_owned()
            }
            PendingConfirmation::Restore(path) => format!(
                "{} から復元します。現在のDBは事前に退避されます。",
                path.display()
            ),
            PendingConfirmation::CloseSaveFailed =>
                "終了前の編集を保存できませんでした。再試行するか、未保存の編集を破棄して終了できます。"
                    .to_owned(),
        };
        div()
            .flex()
            .items_center()
            .gap_3()
            .px_4()
            .py_3()
            .bg(theme::BACKGROUND)
            .border_t_1()
            .border_color(theme::WARNING)
            .child(div().flex_1().child(message))
            .child({
                let entity = cx.entity();
                Button::new("confirm-destructive")
                    .when(!close_save_failed, |button| button.danger())
                    .label(if close_save_failed {
                        "保存を再試行"
                    } else {
                        "実行する"
                    })
                    .on_click(move |_, window, cx| {
                        entity.update(cx, |this, cx| this.confirm_pending(window, cx));
                    })
            })
            .when(close_save_failed, |bar| {
                bar.child({
                    let entity = cx.entity();
                    Button::new("discard-unsaved-close")
                        .danger()
                        .label("変更を破棄して終了")
                        .on_click(move |_, window, cx| {
                            entity.update(cx, |this, _| {
                                this.discard_unsaved_and_close(window);
                            });
                        })
                })
            })
            .child({
                let entity = cx.entity();
                Button::new("cancel-destructive")
                    .label(if close_save_failed {
                        "編集を続ける"
                    } else {
                        "キャンセル"
                    })
                    .on_click(move |_, _, cx| {
                        entity.update(cx, |this, cx| {
                            this.pending_confirmation = None;
                            cx.notify();
                        });
                    })
            })
            .into_any_element()
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let now = OffsetDateTime::now_utc();
        let offset = UtcOffset::current_local_offset().unwrap_or(UtcOffset::UTC);
        let today = now.to_offset(offset).date();
        let today_count = self
            .tasks
            .iter()
            .filter(|task| {
                task.deleted_at.is_none()
                    && !matches!(task.status, TaskStatus::Done | TaskStatus::Archived)
                    && due_is_today(&task.due, today, offset)
            })
            .count();
        let overdue_count = self
            .tasks
            .iter()
            .filter(|task| {
                task.deleted_at.is_none()
                    && !matches!(task.status, TaskStatus::Done | TaskStatus::Archived)
                    && task.due.is_overdue(now, today)
            })
            .count();
        let primary_views = [
            (SmartView::Today, format!("今日  {today_count}")),
            (SmartView::Upcoming, "今後7日".to_owned()),
            (SmartView::Overdue, format!("期限超過  {overdue_count}")),
            (SmartView::All, "すべて".to_owned()),
        ];
        let secondary_views = [
            (SmartView::Undated, "納期未定".to_owned()),
            (SmartView::Doing, "進行中".to_owned()),
            (SmartView::Blocked, "保留".to_owned()),
            (SmartView::Done, "完了".to_owned()),
            (SmartView::Archived, "アーカイブ".to_owned()),
            (SmartView::Trash, "ゴミ箱".to_owned()),
        ];
        div()
            .flex()
            .flex_col()
            .w_full()
            .h_full()
            .flex_shrink_0()
            .bg(theme::SURFACE)
            .border_r_1()
            .border_color(theme::BORDER)
            .overflow_y_scrollbar()
            .p_3()
            .gap_1()
            .child(
                div()
                    .pb_2()
                    .font_weight(FontWeight::BOLD)
                    .text_color(theme::TEXT)
                    .child("ナビゲーション"),
            )
            .child(section_label("スマートビュー"))
            .children(
                primary_views
                    .into_iter()
                    .map(|(view, label)| self.sidebar_button(view, label, cx)),
            )
            .when(self.show_all_smart_views, |sidebar| {
                sidebar.children(
                    secondary_views
                        .into_iter()
                        .map(|(view, label)| self.sidebar_button(view, label, cx)),
                )
            })
            .child({
                let entity = cx.entity();
                Button::new("toggle-secondary-views")
                    .small()
                    .ghost()
                    .label(if self.show_all_smart_views {
                        "表示を減らす"
                    } else {
                        "その他のビュー"
                    })
                    .on_click(move |_, _, cx| {
                        entity.update(cx, |this, cx| {
                            this.show_all_smart_views = !this.show_all_smart_views;
                            cx.notify();
                        });
                    })
            })
            .child({
                let entity = cx.entity();
                Button::new("toggle-saved-views")
                    .small()
                    .ghost()
                    .w_full()
                    .label(if self.show_saved_views {
                        "保存済みビュー ▲"
                    } else {
                        "保存済みビュー ▼"
                    })
                    .on_click(move |_, _, cx| {
                        entity.update(cx, |this, cx| {
                            this.show_saved_views = !this.show_saved_views;
                            cx.notify();
                        });
                    })
            })
            .when(self.show_saved_views, |sidebar| {
                sidebar.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(section_label("保存済みビュー"))
                        .children(
                            self.saved_views
                                .iter()
                                .filter(|view| saved_view_is_available(view))
                                .map(|view| {
                                    let id = view.id;
                                    let entity = cx.entity();
                                    div()
                                        .flex()
                                        .items_center()
                                        .child(self.sidebar_button(
                                            SmartView::Saved(id),
                                            view.name.clone(),
                                            cx,
                                        ))
                                        .child(
                                            Button::new(SharedString::from(format!(
                                                "delete-view-{id}"
                                            )))
                                            .small()
                                            .ghost()
                                            .danger()
                                            .label("×")
                                            .on_click(move |_, _, cx| {
                                                entity.update(cx, |this, cx| {
                                                    this.delete_saved_view(id, cx);
                                                });
                                            }),
                                        )
                                }),
                        )
                        .child(
                            div()
                                .flex()
                                .gap_1()
                                .child(Input::new(&self.view_name_input).small().flex_1())
                                .child(self.small_action_button(
                                    "save-view",
                                    "保存",
                                    cx,
                                    |this, window, cx| {
                                        this.save_current_view(window, cx);
                                    },
                                ))
                                .child(self.small_action_button(
                                    "update-view",
                                    "更新",
                                    cx,
                                    |this, window, cx| {
                                        this.update_active_saved_view(window, cx);
                                    },
                                )),
                        ),
                )
            })
    }

    fn sidebar_button(
        &self,
        view: SmartView,
        label: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let entity = cx.entity();
        Button::new(SharedString::from(format!("sidebar-{view:?}")))
            .ghost()
            .label(label)
            .selected(self.active_view == view)
            .w_full()
            .on_click(move |_, window, cx| {
                entity.update(cx, |this, cx| {
                    this.active_view = view;
                    this.selected_task = None;
                    this.new_task_draft = None;
                    this.sync_management_inputs(view, window, cx);
                    if let SmartView::Saved(id) = view
                        && let Some(saved) = this
                            .saved_views
                            .iter()
                            .find(|saved| saved.id == id)
                            .cloned()
                    {
                        this.view_kind = saved.view_kind;
                        this.filter_statuses = normalized_statuses(&saved.filter.statuses);
                        this.filter_priorities = saved.filter.priorities.iter().copied().collect();
                        this.filter_projects = saved.filter.project_ids.iter().copied().collect();
                        this.filter_unassigned_project = saved.filter.unassigned_project;
                        this.filter_tags = saved.filter.tag_ids.iter().copied().collect();
                        this.filter_match_all_tags = saved.filter.match_all_tags;
                        this.filter_due = saved.filter.due_scope;
                        let offset = UtcOffset::current_local_offset().unwrap_or(UtcOffset::UTC);
                        this.filter_due_from = saved
                            .filter
                            .due_from
                            .map(|date| date.to_offset(offset).date());
                        this.filter_due_to = saved
                            .filter
                            .due_to
                            .map(|date| date.to_offset(offset).date());
                        this.sort = if saved.sort.is_empty() {
                            vec![SortSpec::default()]
                        } else {
                            saved.sort.iter().copied().take(2).collect()
                        };
                        this.group_by = saved.group_by;
                        this.search_input.update(cx, |state, cx| {
                            state.set_value(saved.filter.query, window, cx);
                        });
                        let due_from = this
                            .filter_due_from
                            .map(|date| date.to_string())
                            .unwrap_or_default();
                        let due_to = this
                            .filter_due_to
                            .map(|date| date.to_string())
                            .unwrap_or_default();
                        this.filter_due_from_input.update(cx, |state, cx| {
                            state.set_value(due_from, window, cx);
                        });
                        this.filter_due_to_input.update(cx, |state, cx| {
                            state.set_value(due_to, window, cx);
                        });
                    }
                    cx.notify();
                });
            })
            .into_any_element()
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

    fn render_memo_input(&self) -> AnyElement {
        div()
            .debug_selector(|| "task-memo-input".to_owned())
            .w_full()
            .h(px(160.0))
            .flex_shrink_0()
            .overflow_hidden()
            .child(Input::new(&self.memo_input).h_full())
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
                            .on_click(move |_, _, cx| {
                                entity.update(cx, |this, cx| this.duplicate_task(task_id, cx));
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
                .item(PopupMenuItem::new("複製").on_click(move |_, _, cx| {
                    duplicate_entity.update(cx, |this, cx| this.duplicate_task(task_id, cx));
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

    fn render_board(&self, tasks: Vec<Task>, cx: &mut Context<Self>) -> AnyElement {
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

    fn render_calendar(&self, tasks: Vec<Task>, cx: &mut Context<Self>) -> AnyElement {
        let header = div()
            .flex()
            .items_center()
            .gap_2()
            .px_4()
            .pt_4()
            .when(self.calendar_mode == CalendarMode::Month, |header| {
                header
                    .child({
                        let entity = cx.entity();
                        Button::new("calendar-previous")
                            .small()
                            .label("前月")
                            .on_click(move |_, _, cx| {
                                entity.update(cx, |this, cx| {
                                    this.calendar_month = shift_month(this.calendar_month, -1);
                                    cx.notify();
                                });
                            })
                    })
                    .child(div().font_weight(FontWeight::BOLD).child(format!(
                        "{}年{}月",
                        self.calendar_month.year(),
                        self.calendar_month.month() as u8
                    )))
                    .child({
                        let entity = cx.entity();
                        Button::new("calendar-next").small().label("翌月").on_click(
                            move |_, _, cx| {
                                entity.update(cx, |this, cx| {
                                    this.calendar_month = shift_month(this.calendar_month, 1);
                                    cx.notify();
                                });
                            },
                        )
                    })
                    .child({
                        let entity = cx.entity();
                        Button::new("calendar-current-month")
                            .small()
                            .label("今月")
                            .on_click(move |_, _, cx| {
                                entity.update(cx, |this, cx| {
                                    let now = OffsetDateTime::now_utc();
                                    let offset =
                                        UtcOffset::current_local_offset().unwrap_or(UtcOffset::UTC);
                                    let today = now.to_offset(offset).date();
                                    this.calendar_month =
                                        Date::from_calendar_date(today.year(), today.month(), 1)
                                            .unwrap_or(today);
                                    cx.notify();
                                });
                            })
                    })
            })
            .when(self.calendar_mode == CalendarMode::Agenda, |header| {
                header.child(
                    div()
                        .text_size(px(18.0))
                        .font_weight(FontWeight::BOLD)
                        .child("アジェンダ"),
                )
            })
            .child(div().flex_1())
            .child({
                let entity = cx.entity();
                Button::new("calendar-month")
                    .small()
                    .label("月")
                    .selected(self.calendar_mode == CalendarMode::Month)
                    .on_click(move |_, _, cx| {
                        entity.update(cx, |this, cx| {
                            this.calendar_mode = CalendarMode::Month;
                            cx.notify();
                        });
                    })
            })
            .child({
                let entity = cx.entity();
                Button::new("calendar-agenda")
                    .small()
                    .label("一覧")
                    .selected(self.calendar_mode == CalendarMode::Agenda)
                    .on_click(move |_, _, cx| {
                        entity.update(cx, |this, cx| {
                            this.calendar_mode = CalendarMode::Agenda;
                            cx.notify();
                        });
                    })
            });

        let body = match self.calendar_mode {
            CalendarMode::Month => self.render_calendar_month(tasks, cx),
            CalendarMode::Agenda => self.render_calendar_agenda(tasks, cx),
        };
        div()
            .flex()
            .flex_col()
            .flex_1()
            .h_full()
            .min_h_0()
            .child(header)
            .child(body)
            .into_any_element()
    }

    fn render_calendar_agenda(&self, tasks: Vec<Task>, cx: &mut Context<Self>) -> AnyElement {
        let mut dated = tasks
            .iter()
            .filter(|task| !matches!(task.due, Due::None))
            .cloned()
            .collect::<Vec<_>>();
        dated.sort_by_key(|task| format_due_input(&task.due));
        let undated = tasks
            .iter()
            .filter(|task| matches!(task.due, Due::None))
            .cloned()
            .collect::<Vec<_>>();
        div()
            .flex()
            .flex_1()
            .h_full()
            .gap_4()
            .p_4()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .gap_2()
                    .overflow_y_scrollbar()
                    .child(section_label("納期順"))
                    .children(
                        dated
                            .into_iter()
                            .map(|task| self.render_calendar_item(task, cx)),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .w(px(300.0))
                    .gap_2()
                    .p_3()
                    .rounded_lg()
                    .bg(theme::SURFACE)
                    .child(section_label("納期未定"))
                    .children(
                        undated
                            .into_iter()
                            .map(|task| self.render_calendar_item(task, cx)),
                    ),
            )
            .into_any_element()
    }

    fn render_calendar_month(&self, tasks: Vec<Task>, cx: &mut Context<Self>) -> AnyElement {
        let first = self.calendar_month;
        let now = OffsetDateTime::now_utc();
        let offset = UtcOffset::current_local_offset().unwrap_or(UtcOffset::UTC);
        let today = now.to_offset(offset).date();
        let undated = tasks
            .iter()
            .filter(|task| matches!(task.due, Due::None))
            .cloned()
            .collect::<Vec<_>>();
        let leading = calendar_leading_days(first);
        let days = first.month().length(first.year()) as usize;
        let mut cells = Vec::with_capacity(42);
        for cell in 0..42 {
            if cell < leading || cell >= leading + days {
                cells.push(
                    div()
                        .h(px(CALENDAR_DAY_CELL_HEIGHT))
                        .border_1()
                        .border_color(theme::BORDER)
                        .bg(theme::BACKGROUND)
                        .into_any_element(),
                );
                continue;
            }
            let day = (cell - leading + 1) as u8;
            let date = Date::from_calendar_date(first.year(), first.month(), day)
                .expect("calendar day must be valid");
            let day_tasks = tasks
                .iter()
                .filter(|task| due_date(&task.due) == Some(date))
                .take(3)
                .cloned()
                .collect::<Vec<_>>();
            let total = tasks
                .iter()
                .filter(|task| due_date(&task.due) == Some(date))
                .count();
            cells.push(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .h(px(CALENDAR_DAY_CELL_HEIGHT))
                    .p_2()
                    .border_1()
                    .border_color(if date == today {
                        theme::ACCENT
                    } else {
                        theme::BORDER
                    })
                    .bg(if date == today {
                        theme::SURFACE_HOVER
                    } else {
                        theme::SURFACE
                    })
                    .child(
                        div()
                            .font_weight(FontWeight::BOLD)
                            .text_color(if date == today {
                                theme::ACCENT
                            } else {
                                theme::TEXT
                            })
                            .child(if date == today {
                                format!("{day}  今日")
                            } else {
                                day.to_string()
                            }),
                    )
                    .children(day_tasks.into_iter().map(|task| {
                        let id = task.id;
                        div()
                            .id(SharedString::from(format!("month-{date}-{id}")))
                            .px_1()
                            .rounded_sm()
                            .bg(theme::BACKGROUND)
                            .border_l_2()
                            .border_color(priority_color(task.priority))
                            .text_size(px(12.0))
                            .text_color(if task.status == TaskStatus::Done {
                                theme::MUTED
                            } else {
                                theme::TEXT
                            })
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.select_task(id, window, cx);
                            }))
                            .child(task.title)
                    }))
                    .when(total > 3, |cell| {
                        cell.child(
                            div()
                                .text_size(px(11.0))
                                .text_color(theme::MUTED)
                                .child(format!("ほか{}件", total - 3)),
                        )
                    })
                    .into_any_element(),
            );
        }
        let weekdays = ["日", "月", "火", "水", "木", "金", "土"];
        let calendar = div()
            .debug_selector(|| "calendar-month-grid".to_owned())
            .flex()
            .flex_col()
            .flex_shrink_0()
            .min_h(px(CALENDAR_GRID_MIN_HEIGHT))
            .child(
                div()
                    .grid()
                    .grid_cols(7)
                    .children(weekdays.into_iter().enumerate().map(|(index, day)| {
                        div()
                            .h(px(CALENDAR_WEEKDAY_HEIGHT))
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_center()
                            .text_color(match index {
                                0 => theme::DANGER,
                                6 => theme::ACCENT,
                                _ => theme::MUTED,
                            })
                            .child(day)
                    })),
            )
            .child(div().grid().grid_cols(7).children(cells));
        div()
            .flex()
            .flex_col()
            .flex_1()
            .h_full()
            .min_h_0()
            .gap_3()
            .p_4()
            .overflow_y_scrollbar()
            .child(calendar)
            .child(
                div()
                    .flex()
                    .items_center()
                    .flex_shrink_0()
                    .gap_2()
                    .overflow_x_scrollbar()
                    .child(section_label("納期未定"))
                    .when(undated.is_empty(), |panel| {
                        panel.child(
                            div()
                                .text_size(px(12.0))
                                .text_color(theme::MUTED)
                                .child("納期未定のタスクはありません"),
                        )
                    })
                    .children(undated.into_iter().map(|task| {
                        let id = task.id;
                        div()
                            .id(SharedString::from(format!("undated-{id}")))
                            .px_3()
                            .py_2()
                            .rounded_md()
                            .border_1()
                            .border_color(theme::BORDER)
                            .bg(theme::SURFACE)
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.select_task(id, window, cx);
                            }))
                            .child(task.title)
                    })),
            )
            .into_any_element()
    }

    fn render_calendar_item(&self, task: Task, cx: &mut Context<Self>) -> AnyElement {
        let id = task.id;
        div()
            .id(SharedString::from(format!("calendar-{id}")))
            .flex()
            .items_center()
            .gap_3()
            .p_3()
            .rounded_md()
            .border_1()
            .border_color(theme::BORDER)
            .bg(theme::SURFACE)
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, window, cx| {
                this.select_task(id, window, cx);
            }))
            .child(
                div()
                    .w(px(150.0))
                    .text_color(theme::MUTED)
                    .child(format_due_display(&task.due)),
            )
            .child(task.title)
            .into_any_element()
    }

    fn active_view_label(&self) -> String {
        match self.active_view {
            SmartView::Today => "今日".to_owned(),
            SmartView::Upcoming => "今後7日".to_owned(),
            SmartView::Overdue => "期限超過".to_owned(),
            SmartView::Undated => "納期未定".to_owned(),
            SmartView::All => "すべて".to_owned(),
            SmartView::Doing => "進行中".to_owned(),
            SmartView::Blocked => "保留".to_owned(),
            SmartView::Done => "完了".to_owned(),
            SmartView::Archived => "アーカイブ".to_owned(),
            SmartView::Trash => "ゴミ箱".to_owned(),
            SmartView::Saved(id) => self
                .saved_views
                .iter()
                .find(|view| view.id == id)
                .map(|view| view.name.clone())
                .unwrap_or_else(|| "保存済みビュー".to_owned()),
        }
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let dismiss_error_entity = cx.entity();
        window.set_window_title("HodoQ");
        let bounds = window.bounds();
        let show_task_detail = self.selected_task.is_some() || self.new_task_draft.is_some();
        let resize_workspace = cx.entity();
        let main_panes = h_resizable("workspace-main-panes")
            .with_state(&self.sidebar_resize_state)
            .on_resize(move |state, _, cx| {
                let sizes = state.read(cx).sizes();
                let Some(sidebar_width) = sizes.first().copied() else {
                    return;
                };
                let detail_width = sizes.get(2).copied();
                resize_workspace.update(cx, |this, cx| {
                    this.settings.sidebar_width = f32::from(sidebar_width).clamp(180.0, 380.0);
                    if let Some(detail_width) = detail_width {
                        this.settings.detail_width = f32::from(detail_width).clamp(280.0, 560.0);
                    }
                    cx.notify();
                });
            })
            .child(
                resizable_panel()
                    .size(px(self.settings.sidebar_width))
                    .size_range(px(180.0)..px(380.0))
                    .child(self.render_sidebar(cx)),
            )
            .child(
                resizable_panel()
                    .size_range(px(160.0)..gpui::Pixels::MAX)
                    .child(self.render_content(cx)),
            )
            .when(show_task_detail, |panes| {
                panes.child(
                    resizable_panel()
                        .size(px(self.settings.detail_width))
                        .size_range(px(280.0)..px(560.0))
                        .child(
                            div()
                                .debug_selector(|| "task-detail-slot".to_owned())
                                .size_full()
                                .min_w_0()
                                .overflow_x_hidden()
                                .child(self.render_detail(window, cx)),
                        ),
                )
            });
        self.settings.window.x = Some(f32::from(bounds.origin.x));
        self.settings.window.y = Some(f32::from(bounds.origin.y));
        self.settings.window.width = f32::from(bounds.size.width);
        self.settings.window.height = f32::from(bounds.size.height);
        self.settings.window.maximized = window.is_maximized();
        if let Some(error) = self.worker.take_error() {
            self.error_message = Some(error);
            self.status_message = "保存失敗".to_owned();
        } else if self.status_message == "保存中…" {
            self.status_message = "保存済み".to_owned();
        }
        div()
            .key_context("HodoQ")
            .on_action(cx.listener(|this, _: &NewTaskAction, window, cx| {
                this.open_new_task_form(window, cx);
            }))
            .on_action(cx.listener(|this, _: &SearchAction, window, cx| {
                this.search_input.read(cx).focus_handle(cx).focus(window);
            }))
            .on_action(cx.listener(|this, _: &MoveUpAction, window, cx| {
                this.move_selection(-1, window, cx);
            }))
            .on_action(cx.listener(|this, _: &MoveDownAction, window, cx| {
                this.move_selection(1, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ToggleDoneAction, _, cx| {
                this.toggle_selected_done(cx);
            }))
            .on_action(cx.listener(|this, _: &DeleteAction, _, cx| {
                if let Some(id) = this.selected_task {
                    this.move_to_trash(id, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &CloseDetailAction, _, cx| {
                if this.due_popover_open {
                    this.dismiss_due_popover(cx);
                    return;
                }
                this.close_task_form(cx);
                this.show_command_palette = false;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &CommandPaletteAction, window, cx| {
                this.show_command_palette = !this.show_command_palette;
                if this.show_command_palette {
                    this.command_input.read(cx).focus_handle(cx).focus(window);
                }
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &UndoAction, _, cx| this.undo(cx)))
            .on_action(cx.listener(|this, _: &RedoAction, _, cx| this.redo(cx)))
            .on_action(cx.listener(|this, _: &SelectAllAction, _, cx| {
                this.selection_mode = true;
                this.selected_tasks = this
                    .visible_tasks(cx)
                    .into_iter()
                    .map(|task| task.id)
                    .collect();
                cx.notify();
            }))
            .flex()
            .flex_col()
            .size_full()
            .bg(theme::BACKGROUND)
            .text_color(theme::TEXT)
            .child(self.render_toolbar(cx))
            .child(
                div()
                    .debug_selector(|| "workspace-body".to_owned())
                    .relative()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .child(main_panes),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .h(px(30.0))
                    .px_3()
                    .border_t_1()
                    .border_color(theme::BORDER)
                    .bg(theme::SURFACE)
                    .text_size(px(12.0))
                    .text_color(theme::MUTED)
                    .child(self.status_message.clone())
                    .when_some(self.error_message.clone(), |bar, error| {
                        bar.child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .text_color(theme::DANGER)
                                .child(error)
                                .child(
                                    Button::new("dismiss-error")
                                        .small()
                                        .ghost()
                                        .label("閉じる")
                                        .on_click(move |_, _, cx| {
                                            dismiss_error_entity.update(cx, |this, cx| {
                                                this.error_message = None;
                                                cx.notify();
                                            });
                                        }),
                                ),
                        )
                    }),
            )
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

pub(super) fn schedule_daily_backup(
    worker: &TaskApplication,
    paths: &AppPaths,
) -> Result<(), RepositoryError> {
    let offset = UtcOffset::current_local_offset().unwrap_or(UtcOffset::UTC);
    let today = OffsetDateTime::now_utc().to_offset(offset).date();
    let destination = paths.backups.join(format!("hodoq-{today}.sqlite3"));
    if destination.exists() {
        return Ok(());
    }
    worker.create_backup(destination)?;
    let mut backups = fs::read_dir(&paths.backups)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| is_automatic_backup(path))
        .collect::<Vec<PathBuf>>();
    backups.sort();
    while backups.len() > 5 {
        if let Some(oldest) = backups.first().cloned() {
            fs::remove_file(oldest)?;
            backups.remove(0);
        }
    }
    Ok(())
}

pub(super) fn schedule_maintenance(worker: TaskApplication, paths: AppPaths) {
    let _ = std::thread::Builder::new()
        .name("hodoq-maintenance".to_owned())
        .spawn(move || {
            let _ = schedule_daily_backup(&worker, &paths);
            loop {
                std::thread::sleep(std::time::Duration::from_secs(24 * 60 * 60));
                let _ = schedule_daily_backup(&worker, &paths);
            }
        });
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

fn smart_view_from_setting(value: &str) -> SmartView {
    // Old navigation settings must not reopen the retired management panes.
    if value.starts_with("project:") || value.starts_with("tag:") {
        return SmartView::All;
    }
    if let Some(id) = value.strip_prefix("saved:").and_then(|id| id.parse().ok()) {
        return SmartView::Saved(id);
    }
    match value {
        "inbox" => SmartView::All,
        "upcoming" => SmartView::Upcoming,
        "overdue" => SmartView::Overdue,
        "undated" => SmartView::Undated,
        "all" => SmartView::All,
        "doing" => SmartView::Doing,
        "blocked" => SmartView::Blocked,
        "done" => SmartView::Done,
        "archived" => SmartView::Archived,
        "trash" => SmartView::Trash,
        _ => SmartView::Today,
    }
}

fn smart_view_setting(view: SmartView) -> String {
    match view {
        SmartView::Today => "today",
        SmartView::Upcoming => "upcoming",
        SmartView::Overdue => "overdue",
        SmartView::Undated => "undated",
        SmartView::All => "all",
        SmartView::Doing => "doing",
        SmartView::Blocked => "blocked",
        SmartView::Done => "done",
        SmartView::Archived => "archived",
        SmartView::Trash => "trash",
        SmartView::Saved(id) => return format!("saved:{id}"),
    }
    .to_owned()
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

fn apply_pending_edits(
    tasks: &mut [Task],
    pending_title: Option<&(TaskId, String)>,
    pending_memo: Option<&(TaskId, String)>,
    now: OffsetDateTime,
) -> Result<bool, String> {
    let mut changed = false;
    if let Some((id, title)) = pending_title
        && let Some(task) = tasks.iter_mut().find(|task| task.id == *id)
    {
        task.set_title(title.clone())
            .map_err(|error| error.to_string())?;
        task.touch(now);
        changed = true;
    }
    if let Some((id, memo)) = pending_memo
        && let Some(task) = tasks.iter_mut().find(|task| task.id == *id)
    {
        task.memo = memo.clone();
        task.touch(now);
        changed = true;
    }
    Ok(changed)
}

fn unix_millis() -> i128 {
    OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000
}

fn list_backup_files(directory: &std::path::Path) -> Vec<PathBuf> {
    let mut backups = fs::read_dir(directory)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "sqlite3")
        })
        .collect::<Vec<_>>();
    backups.sort_by(|left, right| right.cmp(left));
    backups.truncate(8);
    backups
}

fn is_automatic_backup(path: &std::path::Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    if name.len() != "hodoq-YYYY-MM-DD.sqlite3".len()
        || !name.starts_with("hodoq-")
        || !name.ends_with(".sqlite3")
    {
        return false;
    }
    let date = &name[6..16];
    date.bytes().enumerate().all(|(index, byte)| match index {
        4 | 7 => byte == b'-',
        _ => byte.is_ascii_digit(),
    })
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
mod tests {
    use super::*;

    #[gpui::test]
    fn workspace_gui_tree_constructs(cx: &mut gpui::TestAppContext) {
        let directory = tempfile::tempdir().unwrap();
        let paths = AppPaths::resolve(Some(directory.path())).unwrap();
        let instance_lock = InstanceLock::acquire(&paths.lock).unwrap();
        let application = TaskApplication::start(&paths.database).unwrap();
        let task = Task::new("中央編集テスト", OffsetDateTime::now_utc()).unwrap();
        let task_id = task.id;
        application.save_task(task).unwrap();
        let snapshot = application.load().unwrap();
        cx.update(gpui_component::init);
        let window = cx.add_window(move |window, cx| {
            Workspace::new(
                application,
                snapshot,
                paths,
                AppSettings::default(),
                instance_lock,
                true,
                window,
                cx,
            )
        });

        window
            .update(cx, |workspace, window, cx| {
                let _tree = workspace.render(window, cx).into_any_element();
                assert_eq!(workspace.active_view, SmartView::All);
                assert_eq!(workspace.view_kind, ViewKind::List);
                assert!(!workspace.show_saved_views);
                workspace.select_task(task_id, window, cx);
                workspace.update_due_from_input("2026-09-05 14:30", window, cx);
                assert!(matches!(
                    workspace.selected_task().unwrap().due,
                    Due::DateTime(_)
                ));
                workspace.clear_due(window, cx);
                assert_eq!(workspace.selected_task().unwrap().due, Due::None);
                workspace.view_kind = ViewKind::Calendar;
                let _selected_calendar_tree = workspace.render(window, cx).into_any_element();
                assert!(workspace.save_and_close_selected_task(cx));
                assert!(workspace.selected_task.is_none());

                workspace.open_new_task_form(window, cx);
                assert!(workspace.new_task_draft.is_some());
                workspace.title_input.update(cx, |state, cx| {
                    state.set_value("新規フォームテスト", window, cx);
                });
                workspace.memo_input.update(cx, |state, cx| {
                    state.set_value("全項目から作成", window, cx);
                });
                workspace.due_input.update(cx, |state, cx| {
                    state.set_value("2026-08-30", window, cx);
                });
                workspace.progress_input.update(cx, |state, cx| {
                    state.set_value("25", window, cx);
                });
                workspace.set_new_task_priority(Priority::High, cx);
                let _new_task_calendar_tree = workspace.render(window, cx).into_any_element();
                assert!(workspace.create_task(cx));
                assert!(workspace.new_task_draft.is_none());
                let created = workspace
                    .tasks
                    .iter()
                    .find(|task| task.title == "新規フォームテスト")
                    .unwrap();
                assert_eq!(created.memo, "全項目から作成");
                assert_eq!(created.priority, Priority::High);
                assert_eq!(created.progress, 25);
                assert!(matches!(created.due, Due::Date(_)));

                workspace.select_task(task_id, window, cx);
                workspace.title_input.update(cx, |state, cx| {
                    state.set_value("", window, cx);
                });
                assert!(!workspace.save_and_close_selected_task(cx));
                assert_eq!(workspace.selected_task, Some(task_id));
                window.remove_window();
            })
            .unwrap();
    }

    #[gpui::test]
    fn task_detail_content_stays_inside_resizable_slot(cx: &mut gpui::TestAppContext) {
        let directory = tempfile::tempdir().unwrap();
        let paths = AppPaths::resolve(Some(directory.path())).unwrap();
        let instance_lock = InstanceLock::acquire(&paths.lock).unwrap();
        let application = TaskApplication::start(&paths.database).unwrap();
        let mut task = Task::new("右端レイアウトテスト", OffsetDateTime::now_utc()).unwrap();
        task.memo = "長いテキスト\n\n".repeat(24);
        let task_id = task.id;
        application.save_task(task).unwrap();
        let snapshot = application.load().unwrap();
        let settings = AppSettings {
            detail_width: 280.0,
            ..AppSettings::default()
        };
        cx.update(gpui_component::init);
        let window = cx.add_window(move |window, cx| {
            let mut workspace = Workspace::new(
                application,
                snapshot,
                paths,
                settings,
                instance_lock,
                true,
                window,
                cx,
            );
            workspace.select_task(task_id, window, cx);
            workspace
        });

        cx.run_until_parked();
        let mut visual = gpui::VisualTestContext::from_window(*window, cx);
        visual.run_until_parked();
        let slot = visual
            .debug_bounds("task-detail-slot")
            .expect("task detail slot should be rendered");
        let workspace_body = visual
            .debug_bounds("workspace-body")
            .expect("workspace body should be rendered");
        let slot_left = f32::from(slot.origin.x);
        let slot_right = f32::from(slot.origin.x + slot.size.width);
        let workspace_right = f32::from(workspace_body.origin.x + workspace_body.size.width);
        assert!(
            (slot_right - workspace_right).abs() <= 0.5,
            "detail slot must end at the window content edge: slot_right={slot_right}px, workspace_right={workspace_right}px"
        );
        for selector in [
            "task-detail-panel",
            "task-memo-input",
            "due-control",
            "task-progress-presets",
        ] {
            let bounds = visual
                .debug_bounds(selector)
                .unwrap_or_else(|| panic!("{selector} should be rendered"));
            assert!(f32::from(bounds.origin.x) >= slot_left - 0.5);
            assert!(
                f32::from(bounds.origin.x + bounds.size.width) <= slot_right + 0.5,
                "{selector} overflowed its slot: right={}px, slot_right={}px",
                f32::from(bounds.origin.x + bounds.size.width),
                slot_right
            );
        }
        let memo = visual
            .debug_bounds("task-memo-input")
            .expect("memo input should be rendered");
        let due = visual
            .debug_bounds("due-control")
            .expect("due control should be rendered");
        assert!(
            f32::from(memo.origin.y + memo.size.height) <= f32::from(due.origin.y),
            "long memo text must not overlap the due control"
        );
        let due_left = f32::from(due.origin.x);
        let due_right = f32::from(due.origin.x + due.size.width);
        let bounds = visual.debug_bounds("due-input-control").unwrap();
        assert!(f32::from(bounds.origin.x) >= due_left - 0.5);
        assert!(f32::from(bounds.origin.x + bounds.size.width) <= due_right + 0.5);
        visual.update(|window, _| window.remove_window());
    }

    #[gpui::test]
    fn legacy_classifications_survive_editing_in_the_simple_workspace(
        cx: &mut gpui::TestAppContext,
    ) {
        let directory = tempfile::tempdir().unwrap();
        let paths = AppPaths::resolve(Some(directory.path())).unwrap();
        let lock = InstanceLock::acquire(&paths.lock).unwrap();
        let now = OffsetDateTime::now_utc();
        let project = Project::new("旧プロジェクト", now);
        let tag = Tag::new("旧タグ", now);
        let mut task = Task::new("分類済みタスク", now).unwrap();
        task.project_id = Some(project.id);
        task.tag_ids = vec![tag.id];
        let view = SavedView {
            id: SavedViewId::new(),
            name: "旧分類ビュー".to_owned(),
            view_kind: ViewKind::List,
            filter: TaskFilter {
                project_ids: vec![project.id],
                ..TaskFilter::default()
            },
            sort: vec![SortSpec::default()],
            group_by: Some(GroupBy::Project),
            sort_order: 0,
            created_at: now,
            updated_at: now,
        };
        let mut repository =
            crate::infrastructure::SqliteRepository::open(&paths.database).unwrap();
        repository.save_project(&project).unwrap();
        repository.save_tag(&tag).unwrap();
        repository.save_task(&task).unwrap();
        repository
            .save_task(&Task::new("分類なしタスク", now).unwrap())
            .unwrap();
        repository.save_view(&view).unwrap();
        drop(repository);
        let application = TaskApplication::start(&paths.database).unwrap();
        let snapshot = application.load().unwrap();
        let original_projects = snapshot.projects.clone();
        let original_tags = snapshot.tags.clone();
        let original_views = snapshot.saved_views.clone();
        assert_eq!(
            smart_view_from_setting(&format!("project:{}", project.id)),
            SmartView::All
        );
        assert_eq!(
            smart_view_from_setting(&format!("tag:{}", tag.id)),
            SmartView::All
        );
        let settings = AppSettings {
            active_view: format!("saved:{}", view.id),
            group_by: Some(GroupBy::Project),
            ..AppSettings::default()
        };
        cx.update(gpui_component::init);
        let window = cx.add_window(move |window, cx| {
            Workspace::new(
                application,
                snapshot,
                paths,
                settings,
                lock,
                false,
                window,
                cx,
            )
        });
        window
            .update(cx, |workspace, window, cx| {
                assert_eq!(workspace.active_view, SmartView::All);
                assert_eq!(workspace.group_by, None);
                assert_eq!(workspace.visible_tasks(cx).len(), 2);
                workspace.select_task(task.id, window, cx);
                workspace.title_input.update(cx, |state, cx| {
                    state.set_value("変更後のタスク", window, cx)
                });
                assert!(workspace.save_and_close_selected_task(cx));
                let reloaded = workspace.worker.load().unwrap();
                let edited = reloaded
                    .tasks
                    .iter()
                    .find(|item| item.id == task.id)
                    .unwrap();
                assert_eq!(edited.title, "変更後のタスク");
                assert_eq!(edited.project_id, task.project_id);
                assert_eq!(edited.tag_ids, task.tag_ids);
                assert_eq!(reloaded.projects, original_projects);
                assert_eq!(reloaded.tags, original_tags);
                assert_eq!(reloaded.saved_views, original_views);
                window.remove_window();
            })
            .unwrap();
    }

    #[gpui::test]
    fn editor_actions_remain_separated_in_a_short_narrow_pane(cx: &mut gpui::TestAppContext) {
        let directory = tempfile::tempdir().unwrap();
        let paths = AppPaths::resolve(Some(directory.path())).unwrap();
        let lock = InstanceLock::acquire(&paths.lock).unwrap();
        let application = TaskApplication::start(&paths.database).unwrap();
        let task = Task::new("ボタンの間隔", OffsetDateTime::now_utc()).unwrap();
        let id = task.id;
        application.save_task(task).unwrap();
        let snapshot = application.load().unwrap();
        cx.update(gpui_component::init);
        let window = cx.add_window(move |window, cx| {
            let mut workspace = Workspace::new(
                application,
                snapshot,
                paths,
                AppSettings {
                    detail_width: 280.0,
                    ..AppSettings::default()
                },
                lock,
                true,
                window,
                cx,
            );
            workspace.select_task(id, window, cx);
            workspace
        });
        cx.run_until_parked();
        let mut visual = gpui::VisualTestContext::from_window(*window, cx);
        visual.simulate_resize(gpui::size(px(900.0), px(600.0)));
        visual.run_until_parked();
        let pane = visual.debug_bounds("task-detail-slot").unwrap();
        visual.simulate_event(gpui::ScrollWheelEvent {
            position: gpui::point(
                pane.origin.x + px(12.0),
                pane.origin.y + pane.size.height - px(12.0),
            ),
            delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.0), px(-2000.0))),
            ..Default::default()
        });
        visual.run_until_parked();
        let save = visual.debug_bounds("task-save-button").unwrap();
        let archive = visual.debug_bounds("task-archive-button").unwrap();
        let trash = visual.debug_bounds("task-trash-button").unwrap();
        for button in [save, archive, trash] {
            assert!(button.size.height >= px(24.0));
            assert!(button.origin.x >= pane.origin.x);
            assert!(button.right() <= pane.right());
            assert!(button.origin.y >= pane.origin.y && button.bottom() <= pane.bottom());
        }
        assert!(
            archive.origin.y - save.bottom() >= px(8.0),
            "save/archive gap: {:?}",
            archive.origin.y - save.bottom()
        );
        assert!(
            trash.origin.y - save.bottom() >= px(8.0),
            "save/trash gap: {:?}",
            trash.origin.y - save.bottom()
        );
        assert!(
            trash.origin.x - archive.right() >= px(8.0)
                || trash.origin.y - archive.bottom() >= px(8.0)
        );
        visual.update(|window, _| window.remove_window());
    }

    #[gpui::test]
    fn list_rows_use_the_same_available_width(cx: &mut gpui::TestAppContext) {
        let directory = tempfile::tempdir().unwrap();
        let paths = AppPaths::resolve(Some(directory.path())).unwrap();
        let instance_lock = InstanceLock::acquire(&paths.lock).unwrap();
        let application = TaskApplication::start(&paths.database).unwrap();
        let now = OffsetDateTime::now_utc();
        let mut undated = Task::new("納期なし", now).unwrap();
        undated.priority = Priority::High;
        let mut dated = Task::new("日付あり", now).unwrap();
        dated.due = Due::Date(Date::from_calendar_date(2026, time::Month::August, 31).unwrap());
        application.save_task(undated).unwrap();
        application.save_task(dated).unwrap();
        let snapshot = application.load().unwrap();
        cx.update(gpui_component::init);
        let window = cx.add_window(move |window, cx| {
            Workspace::new(
                application,
                snapshot,
                paths,
                AppSettings::default(),
                instance_lock,
                true,
                window,
                cx,
            )
        });

        cx.run_until_parked();
        let mut visual = gpui::VisualTestContext::from_window(*window, cx);
        visual.run_until_parked();
        let undated_bounds = visual
            .debug_bounds("task-row-undated")
            .expect("undated task row should be rendered");
        let dated_bounds = visual
            .debug_bounds("task-row-dated")
            .expect("dated task row should be rendered");
        assert!(
            (f32::from(undated_bounds.size.width) - f32::from(dated_bounds.size.width)).abs()
                <= 0.5,
            "list rows must have equal widths: undated={}px, dated={}px",
            f32::from(undated_bounds.size.width),
            f32::from(dated_bounds.size.width)
        );
        visual.update(|window, _| window.remove_window());
    }

    #[gpui::test]
    fn calendar_month_grid_keeps_visible_height(cx: &mut gpui::TestAppContext) {
        let directory = tempfile::tempdir().unwrap();
        let paths = AppPaths::resolve(Some(directory.path())).unwrap();
        let instance_lock = InstanceLock::acquire(&paths.lock).unwrap();
        let application = TaskApplication::start(&paths.database).unwrap();
        let snapshot = application.load().unwrap();
        cx.update(gpui_component::init);
        let window = cx.add_window(move |window, cx| {
            let mut workspace = Workspace::new(
                application,
                snapshot,
                paths,
                AppSettings::default(),
                instance_lock,
                true,
                window,
                cx,
            );
            workspace.view_kind = ViewKind::Calendar;
            workspace
        });

        cx.run_until_parked();
        let mut visual = gpui::VisualTestContext::from_window(*window, cx);
        visual.run_until_parked();
        let bounds = visual
            .debug_bounds("calendar-month-grid")
            .expect("calendar month grid should be rendered");
        assert!(
            f32::from(bounds.size.height) >= CALENDAR_GRID_MIN_HEIGHT,
            "calendar grid height was {}px",
            f32::from(bounds.size.height)
        );
        visual.update(|window, _| window.remove_window());
    }

    #[test]
    fn due_input_supports_none_date_and_datetime() {
        assert_eq!(parse_due("未定").unwrap(), Due::None);
        assert!(matches!(parse_due("2026-08-28").unwrap(), Due::Date(_)));
        assert!(matches!(
            parse_due("2026-08-28 14:30").unwrap(),
            Due::DateTime(_)
        ));
    }

    #[test]
    fn date_picker_selection_updates_due_input() {
        let selected = chrono::NaiveDate::from_ymd_opt(2026, 8, 30).unwrap();
        assert_eq!(
            picker_due_input_value(PickerDate::from(selected), ""),
            "2026-08-30"
        );
        assert_eq!(
            picker_due_input_value(PickerDate::from(selected), "2026-08-20 14:30"),
            "2026-08-30 14:30"
        );
        assert_eq!(
            picker_due_input_value(PickerDate::Single(None), "2026-08-20 14:30"),
            ""
        );
    }

    #[test]
    fn time_selection_updates_the_unified_due_input() {
        assert_eq!(
            due_input_with_time("2026-08-30", Some("14:30")).unwrap(),
            "2026-08-30 14:30"
        );
        assert_eq!(
            due_input_with_time("2026-08-30 14:30", None).unwrap(),
            "2026-08-30"
        );
        assert!(due_input_with_time("", Some("14:30")).is_err());
        assert_eq!(due_time_options().len(), 96);
    }

    #[test]
    fn calendar_month_starts_on_sunday() {
        let sunday = Date::from_calendar_date(2026, time::Month::November, 1).unwrap();
        let saturday = Date::from_calendar_date(2026, time::Month::August, 1).unwrap();
        assert_eq!(calendar_leading_days(sunday), 0);
        assert_eq!(calendar_leading_days(saturday), 6);
    }

    #[test]
    fn automatic_retention_does_not_match_manual_backups() {
        assert!(is_automatic_backup(std::path::Path::new(
            "hodoq-2026-08-28.sqlite3"
        )));
        assert!(!is_automatic_backup(std::path::Path::new(
            "hodoq-manual-123.sqlite3"
        )));
        assert!(!is_automatic_backup(std::path::Path::new(
            "hodoq-before-restore-123.sqlite3"
        )));
    }

    #[test]
    fn pending_edits_are_validated_and_applied_before_close() {
        let now = OffsetDateTime::UNIX_EPOCH;
        let task = Task::new("before", now).unwrap();
        let id = task.id;
        let mut tasks = vec![task];
        let changed = apply_pending_edits(
            &mut tasks,
            Some(&(id, "after".to_owned())),
            Some(&(id, "memo".to_owned())),
            now + time::Duration::seconds(1),
        )
        .unwrap();
        assert!(changed);
        assert_eq!(tasks[0].title, "after");
        assert_eq!(tasks[0].memo, "memo");

        let snapshot = tasks.clone();
        assert!(
            apply_pending_edits(
                &mut tasks,
                Some(&(id, "   ".to_owned())),
                None,
                now + time::Duration::seconds(2),
            )
            .is_err()
        );
        assert_eq!(tasks, snapshot);
    }

    #[test]
    fn ten_thousand_task_visible_search_finds_matching_task() {
        check_ten_thousand_task_visible_search();
    }

    #[test]
    #[ignore = "run performance_ tests in release mode with --test-threads=1"]
    #[allow(clippy::assertions_on_constants)]
    fn performance_ten_thousand_task_visible_search() {
        // An explicit --ignored debug run should fail, not report misleading timings.
        assert!(
            !cfg!(debug_assertions),
            "performance tests require --release"
        );
        let elapsed = check_ten_thousand_task_visible_search();
        eprintln!("10,000 task visible search: {elapsed:?}");
        assert!(
            elapsed < StdDuration::from_millis(100),
            "10,000 task visible search took {elapsed:?}"
        );
    }

    fn check_ten_thousand_task_visible_search() -> StdDuration {
        let now = OffsetDateTime::UNIX_EPOCH;
        let tasks = (0..10_000)
            .map(|index| Task::new(format!("task {index}"), now).unwrap())
            .collect::<Vec<_>>();
        let started = std::time::Instant::now();
        let mut matches = tasks
            .iter()
            .filter(|task| {
                task.title.to_lowercase().contains("task 9999")
                    || task.memo.to_lowercase().contains("task 9999")
            })
            .cloned()
            .collect::<Vec<_>>();
        matches.sort_by(|left, right| {
            compare_tasks(left, right, &[SortSpec::default()], UtcOffset::UTC)
        });
        let elapsed = started.elapsed();

        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].id, tasks[9999].id);
        elapsed
    }
}
