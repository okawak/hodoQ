use std::{cmp::Ordering, collections::HashSet, fs, path::PathBuf, time::Duration as StdDuration};

use gpui::{
    AnyElement, AppContext as _, Context, Entity, Focusable as _, FontWeight,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, Rgba, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, Timer, Window, div,
    prelude::FluentBuilder as _, px, uniform_list,
};
use gpui_component::{
    Selectable as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    input::{Input, InputEvent, InputState},
    menu::{ContextMenuExt as _, PopupMenuItem},
    progress::Progress,
    resizable::{ResizableState, h_resizable, resizable_panel},
    scroll::ScrollableElement as _,
};
use time::{Date, OffsetDateTime, PrimitiveDateTime, UtcOffset, macros::format_description};

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
    Inbox,
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
    Project(ProjectId),
    Tag(TagId),
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
    Inbox,
    Today,
    All,
    StatusInbox,
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

#[derive(Clone)]
struct TaskDrag {
    id: TaskId,
    title: String,
}

#[derive(Clone)]
enum VirtualListItem {
    Group(String),
    Task(Task),
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
    show_more_menu: bool,
    show_all_smart_views: bool,
    show_task_editor_details: bool,
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
    new_task_input: Entity<InputState>,
    search_input: Entity<InputState>,
    command_input: Entity<InputState>,
    title_input: Entity<InputState>,
    memo_input: Entity<InputState>,
    due_input: Entity<InputState>,
    progress_input: Entity<InputState>,
    project_input: Entity<InputState>,
    project_description_input: Entity<InputState>,
    project_color_input: Entity<InputState>,
    tag_input: Entity<InputState>,
    tag_color_input: Entity<InputState>,
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
        let new_task_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("タスク名を入力して保存"));
        let search_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("タイトル・メモを検索"));
        let command_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("操作を検索（例: 今日、完了、バックアップ）")
        });
        let title_input = cx.new(|cx| InputState::new(window, cx).placeholder("タイトル"));
        let memo_input = cx.new(|cx| {
            InputState::new(window, cx)
                .auto_grow(6, 18)
                .placeholder("メモ")
        });
        let due_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("未定 / YYYY-MM-DD / YYYY-MM-DD HH:MM")
        });
        let progress_input = cx.new(|cx| InputState::new(window, cx).placeholder("進捗 0〜100"));
        let project_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("新しいプロジェクト"));
        let project_description_input = cx.new(|cx| {
            InputState::new(window, cx)
                .auto_grow(3, 8)
                .placeholder("プロジェクトの説明")
        });
        let project_color_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("色（例: #2f81f7）"));
        let tag_input = cx.new(|cx| InputState::new(window, cx).placeholder("新しいタグ"));
        let tag_color_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("色（例: #3fb950）"));
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
            cx.subscribe_in(
                &new_task_input,
                window,
                |this, _, event: &InputEvent, window, cx| {
                    if matches!(event, InputEvent::PressEnter { .. }) {
                        this.create_task(window, cx);
                    }
                },
            ),
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
                &due_input,
                window,
                |this, state, event: &InputEvent, _window, cx| {
                    if matches!(event, InputEvent::PressEnter { .. }) {
                        this.update_selected_due(state.read(cx).value().as_str(), cx);
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
                &project_input,
                window,
                |this, _, event: &InputEvent, window, cx| {
                    if matches!(event, InputEvent::PressEnter { .. }) {
                        if matches!(this.active_view, SmartView::Project(_)) {
                            this.update_active_project(window, cx);
                        } else {
                            this.create_project(window, cx);
                        }
                    }
                },
            ),
            cx.subscribe_in(
                &tag_input,
                window,
                |this, _, event: &InputEvent, window, cx| {
                    if matches!(event, InputEvent::PressEnter { .. }) {
                        if matches!(this.active_view, SmartView::Tag(_)) {
                            this.update_active_tag(window, cx);
                        } else {
                            this.create_tag(window, cx);
                        }
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
        let mut initial_group_by = settings.group_by;
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
            SmartView::Inbox
        } else {
            smart_view_from_setting(&settings.active_view)
        };
        initial_active_view = match initial_active_view {
            SmartView::Project(id) if !snapshot.projects.iter().any(|project| project.id == id) => {
                SmartView::All
            }
            SmartView::Tag(id) if !snapshot.tags.iter().any(|tag| tag.id == id) => SmartView::All,
            SmartView::Saved(id) if !snapshot.saved_views.iter().any(|view| view.id == id) => {
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
            initial_filter_statuses = saved.filter.statuses.iter().copied().collect();
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
        match initial_active_view {
            SmartView::Project(id) => {
                if let Some(project) = snapshot.projects.iter().find(|project| project.id == id) {
                    project_input.update(cx, |state, cx| {
                        state.set_value(project.name.clone(), window, cx);
                    });
                    project_description_input.update(cx, |state, cx| {
                        state.set_value(project.description.clone(), window, cx);
                    });
                    project_color_input.update(cx, |state, cx| {
                        state.set_value(project.color.clone().unwrap_or_default(), window, cx);
                    });
                }
            }
            SmartView::Tag(id) => {
                if let Some(tag) = snapshot.tags.iter().find(|tag| tag.id == id) {
                    tag_input.update(cx, |state, cx| {
                        state.set_value(tag.name.clone(), window, cx);
                    });
                    tag_color_input.update(cx, |state, cx| {
                        state.set_value(tag.color.clone().unwrap_or_default(), window, cx);
                    });
                }
            }
            SmartView::Saved(id) => {
                if let Some(saved) = snapshot.saved_views.iter().find(|view| view.id == id) {
                    view_name_input.update(cx, |state, cx| {
                        state.set_value(saved.name.clone(), window, cx);
                    });
                }
            }
            _ => {}
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
            show_more_menu: false,
            show_all_smart_views: false,
            show_task_editor_details: false,
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
            new_task_input,
            search_input,
            command_input,
            title_input,
            memo_input,
            due_input,
            progress_input,
            project_input,
            project_description_input,
            project_color_input,
            tag_input,
            tag_color_input,
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

    fn create_task(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let title = self.new_task_input.read(cx).value().to_string();
        match Task::new(title, OffsetDateTime::now_utc()) {
            Ok(mut task) => {
                task.sort_order = self
                    .tasks
                    .iter()
                    .map(|task| task.sort_order)
                    .max()
                    .unwrap_or_default()
                    + 1024;
                if let Err(error) = self.worker.save_task(task.clone()) {
                    self.set_error(error);
                    return;
                }
                self.push_task_history(vec![(None, Some(task.clone()))]);
                self.selected_task = Some(task.id);
                self.tasks.push(task);
                self.new_task_input.update(cx, |state, cx| {
                    state.set_value("", window, cx);
                });
                self.sync_detail_inputs(window, cx);
                self.status_message = "新しいタスクを保存しました".to_owned();
                cx.notify();
            }
            Err(error) => {
                self.error_message = Some(error.to_string());
                cx.notify();
            }
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

    fn move_task_order(&mut self, id: TaskId, direction: i32, cx: &mut Context<Self>) {
        let mut indices = self
            .tasks
            .iter()
            .enumerate()
            .filter(|(_, task)| task.deleted_at.is_none())
            .map(|(index, task)| (index, task.sort_order))
            .collect::<Vec<_>>();
        indices.sort_by_key(|(_, order)| *order);
        let Some(position) = indices
            .iter()
            .position(|(index, _)| self.tasks[*index].id == id)
        else {
            return;
        };
        let target = if direction < 0 {
            position.checked_sub(1)
        } else if position + 1 < indices.len() {
            Some(position + 1)
        } else {
            None
        };
        let Some(target) = target else { return };
        let left = indices[position].0;
        let right = indices[target].0;
        let before_left = self.tasks[left].clone();
        let before_right = self.tasks[right].clone();
        let old = self.tasks[left].sort_order;
        self.tasks[left].sort_order = self.tasks[right].sort_order;
        self.tasks[right].sort_order = old;
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
            self.status_message = "表示順を変更しました".to_owned();
        }
        cx.notify();
    }

    fn swap_task_order(&mut self, dragged: TaskId, target: TaskId, cx: &mut Context<Self>) {
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
            self.status_message = "ドラッグで表示順を変更しました".to_owned();
        }
        cx.notify();
    }

    fn move_project_order(&mut self, id: ProjectId, direction: i32, cx: &mut Context<Self>) {
        let mut indices = (0..self.projects.len()).collect::<Vec<_>>();
        indices.sort_by_key(|index| self.projects[*index].sort_order);
        let Some(position) = indices
            .iter()
            .position(|index| self.projects[*index].id == id)
        else {
            return;
        };
        let target = if direction < 0 {
            position.checked_sub(1)
        } else if position + 1 < indices.len() {
            Some(position + 1)
        } else {
            None
        };
        let Some(target) = target else { return };
        let left = indices[position];
        let right = indices[target];
        let before_left = self.projects[left].clone();
        let before_right = self.projects[right].clone();
        let order = self.projects[left].sort_order;
        self.projects[left].sort_order = self.projects[right].sort_order;
        self.projects[right].sort_order = order;
        self.projects[left].updated_at = OffsetDateTime::now_utc();
        self.projects[right].updated_at = OffsetDateTime::now_utc();
        let after_left = self.projects[left].clone();
        let after_right = self.projects[right].clone();
        let result = self
            .worker
            .save_projects(vec![after_left.clone(), after_right.clone()]);
        if let Err(error) = result {
            self.projects[left] = before_left;
            self.projects[right] = before_right;
            self.set_error(error);
        } else {
            self.push_history(HistoryEntry {
                task_changes: Vec::new(),
                project_changes: vec![
                    (Some(before_left), Some(after_left)),
                    (Some(before_right), Some(after_right)),
                ],
                tag_changes: Vec::new(),
            });
            self.status_message = "プロジェクト順を変更しました".to_owned();
        }
        cx.notify();
    }

    fn create_project(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.project_input.read(cx).value().to_string();
        if name.trim().is_empty() || name.chars().count() > 100 {
            self.error_message = Some("プロジェクト名は1〜100文字で入力してください".to_owned());
            cx.notify();
            return;
        }
        let mut project = Project::new(name, OffsetDateTime::now_utc());
        project.sort_order = self.projects.len() as i64 * 1024;
        if let Err(error) = self.worker.save_project(project.clone()) {
            self.set_error(error);
            return;
        }
        self.push_history(HistoryEntry {
            task_changes: Vec::new(),
            project_changes: vec![(None, Some(project.clone()))],
            tag_changes: Vec::new(),
        });
        self.projects.push(project);
        self.project_input
            .update(cx, |state, cx| state.set_value("", window, cx));
        cx.notify();
    }

    fn update_active_project(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let SmartView::Project(id) = self.active_view else {
            return;
        };
        let name = self.project_input.read(cx).value().trim().to_owned();
        let description = self.project_description_input.read(cx).value().to_string();
        let color = self.project_color_input.read(cx).value().trim().to_owned();
        if name.is_empty() || name.chars().count() > 100 || description.chars().count() > 5000 {
            self.error_message = Some("プロジェクト名または説明が長すぎます".to_owned());
            cx.notify();
            return;
        }
        if !color.is_empty() && !is_hex_color(&color) {
            self.error_message = Some("色は #RRGGBB 形式で入力してください".to_owned());
            cx.notify();
            return;
        }
        let Some(index) = self.projects.iter().position(|project| project.id == id) else {
            return;
        };
        let before = self.projects[index].clone();
        let project = &mut self.projects[index];
        project.name = name;
        project.description = description;
        project.color = (!color.is_empty()).then_some(color);
        project.updated_at = OffsetDateTime::now_utc();
        let after = project.clone();
        if let Err(error) = self.worker.save_project(after.clone()) {
            self.projects[index] = before;
            self.set_error(error);
            return;
        }
        self.push_history(HistoryEntry {
            task_changes: Vec::new(),
            project_changes: vec![(Some(before), Some(after))],
            tag_changes: Vec::new(),
        });
        self.error_message = None;
        self.status_message = "プロジェクトを更新しました".to_owned();
        cx.notify();
    }

    fn create_tag(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.tag_input.read(cx).value().to_string();
        if name.trim().is_empty() || name.chars().count() > 50 {
            self.error_message = Some("タグ名は1〜50文字で入力してください".to_owned());
            cx.notify();
            return;
        }
        if self
            .tags
            .iter()
            .any(|tag| tag.name.eq_ignore_ascii_case(name.trim()))
        {
            self.error_message = Some("同じ名前のタグが既にあります".to_owned());
            cx.notify();
            return;
        }
        let tag = Tag::new(name, OffsetDateTime::now_utc());
        if let Err(error) = self.worker.save_tag(tag.clone()) {
            self.set_error(error);
            return;
        }
        self.push_history(HistoryEntry {
            task_changes: Vec::new(),
            project_changes: Vec::new(),
            tag_changes: vec![(None, Some(tag.clone()))],
        });
        self.tags.push(tag);
        self.tag_input
            .update(cx, |state, cx| state.set_value("", window, cx));
        cx.notify();
    }

    fn update_active_tag(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let SmartView::Tag(id) = self.active_view else {
            return;
        };
        let name = self.tag_input.read(cx).value().trim().to_owned();
        let color = self.tag_color_input.read(cx).value().trim().to_owned();
        if name.is_empty()
            || name.chars().count() > 50
            || self
                .tags
                .iter()
                .any(|tag| tag.id != id && tag.name.eq_ignore_ascii_case(&name))
        {
            self.error_message = Some("タグ名が不正か、同じ名前のタグが既にあります".to_owned());
            cx.notify();
            return;
        }
        if !color.is_empty() && !is_hex_color(&color) {
            self.error_message = Some("色は #RRGGBB 形式で入力してください".to_owned());
            cx.notify();
            return;
        }
        let Some(index) = self.tags.iter().position(|tag| tag.id == id) else {
            return;
        };
        let before = self.tags[index].clone();
        let tag = &mut self.tags[index];
        tag.name = name;
        tag.color = (!color.is_empty()).then_some(color);
        let after = tag.clone();
        if let Err(error) = self.worker.save_tag(after.clone()) {
            self.tags[index] = before;
            self.set_error(error);
            return;
        }
        self.push_history(HistoryEntry {
            task_changes: Vec::new(),
            project_changes: Vec::new(),
            tag_changes: vec![(Some(before), Some(after))],
        });
        self.error_message = None;
        self.status_message = "タグを更新しました".to_owned();
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
            SmartView::Inbox => Some(SavedBaseView::Inbox),
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
            SmartView::Project(id) => Some(SavedBaseView::Project(id)),
            SmartView::Tag(id) => Some(SavedBaseView::Tag(id)),
            SmartView::Saved(id) => self
                .saved_views
                .iter()
                .find(|view| view.id == id)
                .and_then(|view| view.filter.base_view),
        }
    }

    fn delete_project(&mut self, id: ProjectId, cx: &mut Context<Self>) {
        let Some(project) = self
            .projects
            .iter()
            .find(|project| project.id == id)
            .cloned()
        else {
            return;
        };
        if let Err(error) = self.worker.delete_project(id) {
            self.set_error(error);
            return;
        }
        self.projects.retain(|project| project.id != id);
        let mut task_changes = Vec::new();
        for task in &mut self.tasks {
            if task.project_id == Some(id) {
                let before = task.clone();
                task.project_id = None;
                task_changes.push((Some(before), Some(task.clone())));
            }
        }
        self.push_history(HistoryEntry {
            task_changes,
            project_changes: vec![(Some(project), None)],
            tag_changes: Vec::new(),
        });
        if self.active_view == SmartView::Project(id) {
            self.active_view = SmartView::All;
        }
        self.status_message = "プロジェクトを削除しました".to_owned();
        cx.notify();
    }

    fn toggle_project_archive(&mut self, id: ProjectId, cx: &mut Context<Self>) {
        let Some(index) = self.projects.iter().position(|project| project.id == id) else {
            return;
        };
        let before = self.projects[index].clone();
        let project = &mut self.projects[index];
        let now = OffsetDateTime::now_utc();
        project.archived_at = if project.archived_at.is_some() {
            None
        } else {
            Some(now)
        };
        project.updated_at = now;
        let after = project.clone();
        if let Err(error) = self.worker.save_project(after.clone()) {
            self.projects[index] = before;
            self.set_error(error);
        } else {
            self.push_history(HistoryEntry {
                task_changes: Vec::new(),
                project_changes: vec![(Some(before), Some(after))],
                tag_changes: Vec::new(),
            });
            self.status_message = "プロジェクトのアーカイブ状態を更新しました".to_owned();
        }
        cx.notify();
    }

    fn project_summary(&self, project: &Project) -> String {
        let now = OffsetDateTime::now_utc();
        let offset = UtcOffset::current_local_offset().unwrap_or(UtcOffset::UTC);
        let today = now.to_offset(offset).date();
        let tasks = self
            .tasks
            .iter()
            .filter(|task| task.project_id == Some(project.id) && task.deleted_at.is_none())
            .collect::<Vec<_>>();
        let open = tasks
            .iter()
            .filter(|task| !matches!(task.status, TaskStatus::Done | TaskStatus::Archived))
            .count();
        let overdue = tasks
            .iter()
            .filter(|task| task.due.is_overdue(now, today) && task.status != TaskStatus::Done)
            .count();
        let average = if tasks.is_empty() {
            0
        } else {
            tasks
                .iter()
                .map(|task| usize::from(task.progress))
                .sum::<usize>()
                / tasks.len()
        };
        format!(
            "{}{} · 未完了{open} · 超過{overdue} · 平均{average}%",
            project.name,
            if project.archived_at.is_some() {
                "（アーカイブ）"
            } else {
                ""
            }
        )
    }

    fn delete_tag(&mut self, id: TagId, cx: &mut Context<Self>) {
        let Some(tag) = self.tags.iter().find(|tag| tag.id == id).cloned() else {
            return;
        };
        if let Err(error) = self.worker.delete_tag(id) {
            self.set_error(error);
            return;
        }
        self.tags.retain(|tag| tag.id != id);
        let mut task_changes = Vec::new();
        for task in &mut self.tasks {
            let before = task.clone();
            task.tag_ids.retain(|tag_id| *tag_id != id);
            if before != *task {
                task_changes.push((Some(before), Some(task.clone())));
            }
        }
        self.push_history(HistoryEntry {
            task_changes,
            project_changes: Vec::new(),
            tag_changes: vec![(Some(tag), None)],
        });
        if self.active_view == SmartView::Tag(id) {
            self.active_view = SmartView::All;
        }
        self.status_message = "タグを削除しました".to_owned();
        cx.notify();
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

    fn bulk_project(&mut self, project_id: Option<ProjectId>, cx: &mut Context<Self>) {
        self.bulk_update(cx, |task, now| {
            task.project_id = project_id;
            task.touch(now);
        });
    }

    fn bulk_set_tag(&mut self, tag_id: TagId, selected: bool, cx: &mut Context<Self>) {
        self.bulk_update(cx, |task, now| {
            if selected && !task.tag_ids.contains(&tag_id) {
                task.tag_ids.push(tag_id);
            } else if !selected {
                task.tag_ids.retain(|id| *id != tag_id);
            }
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
        self.show_task_editor_details = false;
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
        for input in [
            &self.project_input,
            &self.project_description_input,
            &self.project_color_input,
            &self.tag_input,
            &self.tag_color_input,
            &self.view_name_input,
        ] {
            input.update(cx, |state, cx| state.set_value("", window, cx));
        }
        match view {
            SmartView::Project(id) => {
                if let Some(project) = self.projects.iter().find(|project| project.id == id) {
                    let project = project.clone();
                    self.project_input.update(cx, |state, cx| {
                        state.set_value(project.name, window, cx);
                    });
                    self.project_description_input.update(cx, |state, cx| {
                        state.set_value(project.description, window, cx);
                    });
                    self.project_color_input.update(cx, |state, cx| {
                        state.set_value(project.color.unwrap_or_default(), window, cx);
                    });
                }
            }
            SmartView::Tag(id) => {
                if let Some(tag) = self.tags.iter().find(|tag| tag.id == id) {
                    let tag = tag.clone();
                    self.tag_input.update(cx, |state, cx| {
                        state.set_value(tag.name, window, cx);
                    });
                    self.tag_color_input.update(cx, |state, cx| {
                        state.set_value(tag.color.unwrap_or_default(), window, cx);
                    });
                }
            }
            SmartView::Saved(id) => {
                if let Some(view) = self.saved_views.iter().find(|view| view.id == id) {
                    let name = view.name.clone();
                    self.view_name_input.update(cx, |state, cx| {
                        state.set_value(name, window, cx);
                    });
                }
            }
            _ => {}
        }
    }

    fn sync_detail_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(task) = self.selected_task().cloned() else {
            return;
        };
        self.title_input.update(cx, |state, cx| {
            state.set_value(task.title, window, cx);
        });
        self.memo_input.update(cx, |state, cx| {
            state.set_value(task.memo, window, cx);
        });
        self.due_input.update(cx, |state, cx| {
            state.set_value(format_due_input(&task.due), window, cx);
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

    fn update_selected_due(&mut self, value: &str, cx: &mut Context<Self>) {
        match parse_due(value) {
            Ok(due) => {
                self.update_selected_task(cx, |task, now| {
                    task.due = due;
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
            Ok(due) => due,
            Err(message) => {
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
        self.show_task_editor_details = false;
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

    fn set_task_project(
        &mut self,
        id: TaskId,
        project_id: Option<ProjectId>,
        cx: &mut Context<Self>,
    ) {
        self.update_task(id, cx, |task, now| {
            task.project_id = project_id;
            task.touch(now);
        });
    }

    fn toggle_task_tag(&mut self, id: TaskId, tag_id: TagId, cx: &mut Context<Self>) {
        self.update_task(id, cx, |task, now| {
            if let Some(index) = task.tag_ids.iter().position(|existing| *existing == tag_id) {
                task.tag_ids.remove(index);
            } else {
                task.tag_ids.push(tag_id);
            }
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
                    SmartView::Saved(id) => self
                        .saved_views
                        .iter()
                        .find(|view| view.id == id)
                        .is_some_and(|view| task_matches_saved_view(task, view)),
                    _ if task.deleted_at.is_some() => false,
                    SmartView::Archived => task.status == TaskStatus::Archived,
                    _ if task.status == TaskStatus::Archived => false,
                    SmartView::Inbox => task.status == TaskStatus::Inbox,
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
                    SmartView::Project(id) => task.project_id == Some(id),
                    SmartView::Tag(id) => task.tag_ids.contains(&id),
                }
            })
            .cloned()
            .collect::<Vec<_>>();
        tasks.sort_by(|left, right| compare_tasks(left, right, &self.sort));
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
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_size(px(12.0))
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(theme::MUTED)
                                    .child("新規タスク"),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(Input::new(&self.new_task_input).w(px(280.0)))
                                    .child(
                                        Button::new("add-task")
                                            .primary()
                                            .label("タスクを保存")
                                            .on_click(move |_, window, cx| {
                                                entity.update(cx, |this, cx| {
                                                    this.create_task(window, cx);
                                                });
                                            }),
                                    ),
                            ),
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
            (PaletteCommand::Inbox, "受信箱へ移動", "受信箱 inbox view"),
            (PaletteCommand::Today, "今日へ移動", "今日 today view"),
            (PaletteCommand::All, "すべてへ移動", "全部 all view"),
            (
                PaletteCommand::StatusInbox,
                "選択タスクを受信箱へ",
                "状態 inbox 受信箱",
            ),
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
            PaletteCommand::NewTask => self.new_task_input.read(cx).focus_handle(cx).focus(window),
            PaletteCommand::Search => self.search_input.read(cx).focus_handle(cx).focus(window),
            PaletteCommand::Inbox => self.active_view = SmartView::Inbox,
            PaletteCommand::Today => self.active_view = SmartView::Today,
            PaletteCommand::All => self.active_view = SmartView::All,
            PaletteCommand::StatusInbox => self.set_selected_task_status(TaskStatus::Inbox, cx),
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
            .map(|panel| {
                panel
                    .child(div().ml_3().text_color(theme::MUTED).child("プロジェクト"))
                    .child({
                        let entity = cx.entity();
                        Button::new("filter-project-none")
                            .small()
                            .label("なし")
                            .selected(self.filter_unassigned_project)
                            .on_click(move |_, _, cx| {
                                entity.update(cx, |this, cx| {
                                    this.filter_unassigned_project =
                                        !this.filter_unassigned_project;
                                    cx.notify();
                                });
                            })
                    })
                    .children(self.projects.iter().map(|project| {
                        let id = project.id;
                        let entity = cx.entity();
                        Button::new(SharedString::from(format!("filter-project-{id}")))
                            .small()
                            .label(project.name.clone())
                            .selected(self.filter_projects.contains(&id))
                            .on_click(move |_, _, cx| {
                                entity.update(cx, |this, cx| {
                                    if !this.filter_projects.insert(id) {
                                        this.filter_projects.remove(&id);
                                    }
                                    cx.notify();
                                });
                            })
                    }))
            })
            .when(!self.tags.is_empty(), |panel| {
                panel
                    .child(div().ml_3().text_color(theme::MUTED).child("タグ"))
                    .children(self.tags.iter().map(|tag| {
                        let id = tag.id;
                        let entity = cx.entity();
                        Button::new(SharedString::from(format!("filter-tag-{id}")))
                            .small()
                            .label(tag.name.clone())
                            .selected(self.filter_tags.contains(&id))
                            .on_click(move |_, _, cx| {
                                entity.update(cx, |this, cx| {
                                    if !this.filter_tags.insert(id) {
                                        this.filter_tags.remove(&id);
                                    }
                                    cx.notify();
                                });
                            })
                    }))
                    .child({
                        let entity = cx.entity();
                        Button::new("filter-tag-mode")
                            .small()
                            .label(if self.filter_match_all_tags {
                                "AND"
                            } else {
                                "OR"
                            })
                            .on_click(move |_, _, cx| {
                                entity.update(cx, |this, cx| {
                                    this.filter_match_all_tags = !this.filter_match_all_tags;
                                    cx.notify();
                                });
                            })
                    })
            })
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
                    (Some(GroupBy::Project), "プロジェクト"),
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
            .child(div().text_color(theme::MUTED).child("プロジェクト"))
            .child({
                let entity = cx.entity();
                Button::new("bulk-project-none")
                    .small()
                    .label("なし")
                    .on_click(move |_, _, cx| {
                        entity.update(cx, |this, cx| this.bulk_project(None, cx));
                    })
            })
            .children(self.projects.iter().map(|project| {
                let id = project.id;
                let entity = cx.entity();
                Button::new(SharedString::from(format!("bulk-project-{id}")))
                    .small()
                    .label(project.name.clone())
                    .on_click(move |_, _, cx| {
                        entity.update(cx, |this, cx| this.bulk_project(Some(id), cx));
                    })
            }))
            .child(div().text_color(theme::MUTED).child("タグ"))
            .children(self.tags.iter().flat_map(|tag| {
                let id = tag.id;
                let add_entity = cx.entity();
                let remove_entity = cx.entity();
                [
                    Button::new(SharedString::from(format!("bulk-tag-add-{id}")))
                        .small()
                        .label(format!("＋#{}", tag.name))
                        .on_click(move |_, _, cx| {
                            add_entity.update(cx, |this, cx| {
                                this.bulk_set_tag(id, true, cx);
                            });
                        }),
                    Button::new(SharedString::from(format!("bulk-tag-remove-{id}")))
                        .small()
                        .label(format!("−#{}", tag.name))
                        .on_click(move |_, _, cx| {
                            remove_entity.update(cx, |this, cx| {
                                this.bulk_set_tag(id, false, cx);
                            });
                        }),
                ]
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
            (SmartView::Inbox, "受信箱".to_owned()),
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
                    .flex()
                    .items_center()
                    .justify_between()
                    .pb_2()
                    .child(
                        div()
                            .font_weight(FontWeight::BOLD)
                            .text_color(theme::TEXT)
                            .child("ナビゲーション"),
                    )
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(theme::MUTED)
                            .child("右端をドラッグ ↔"),
                    ),
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
            .child(section_label("プロジェクト"))
            .children(self.projects.iter().map(|project| {
                let id = project.id;
                div()
                    .flex()
                    .items_center()
                    .when_some(
                        project.color.as_deref().and_then(parse_hex_color),
                        |row, color| {
                            row.child(div().w(px(8.0)).h(px(8.0)).rounded_full().bg(color))
                        },
                    )
                    .child(self.sidebar_button(
                        SmartView::Project(id),
                        self.project_summary(project),
                        cx,
                    ))
            }))
            .child(
                div()
                    .flex()
                    .gap_1()
                    .child(Input::new(&self.project_input).small().flex_1())
                    .child(self.small_action_button(
                        "add-project",
                        "＋",
                        cx,
                        |this, window, cx| {
                            this.create_project(window, cx);
                        },
                    )),
            )
            .child(section_label("タグ"))
            .children(self.tags.iter().map(|tag| {
                let id = tag.id;
                div()
                    .flex()
                    .items_center()
                    .when_some(
                        tag.color.as_deref().and_then(parse_hex_color),
                        |row, color| {
                            row.child(div().w(px(8.0)).h(px(8.0)).rounded_full().bg(color))
                        },
                    )
                    .child(self.sidebar_button(SmartView::Tag(id), format!("#{}", tag.name), cx))
            }))
            .child(
                div()
                    .flex()
                    .gap_1()
                    .child(Input::new(&self.tag_input).small().flex_1())
                    .child(
                        self.small_action_button("add-tag", "＋", cx, |this, window, cx| {
                            this.create_tag(window, cx);
                        }),
                    ),
            )
            .child(section_label("保存済みビュー"))
            .children(self.saved_views.iter().map(|view| {
                let id = view.id;
                let entity = cx.entity();
                div()
                    .flex()
                    .items_center()
                    .child(self.sidebar_button(SmartView::Saved(id), view.name.clone(), cx))
                    .child(
                        Button::new(SharedString::from(format!("delete-view-{id}")))
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
            }))
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
            )
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
                    this.sync_management_inputs(view, window, cx);
                    if let SmartView::Saved(id) = view
                        && let Some(saved) = this
                            .saved_views
                            .iter()
                            .find(|saved| saved.id == id)
                            .cloned()
                    {
                        this.view_kind = saved.view_kind;
                        this.filter_statuses = saved.filter.statuses.iter().copied().collect();
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
            .when(self.selected_task.is_some(), |content| {
                content.child(self.render_center_task_editor(cx))
            })
            .child(view)
            .into_any_element()
    }

    fn render_center_task_editor(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(task) = self.selected_task().cloned() else {
            return div().into_any_element();
        };
        let id = task.id;
        div()
            .flex()
            .flex_col()
            .flex_shrink_0()
            .gap_3()
            .px_4()
            .py_3()
            .bg(theme::SURFACE)
            .border_b_1()
            .border_color(theme::BORDER)
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(
                                div()
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(theme::TEXT)
                                    .child("タスクを編集"),
                            )
                            .child(div().text_size(px(12.0)).text_color(theme::MUTED).child(
                                format!(
                                    "{} · 優先度 {} · {}%",
                                    task.status.label(),
                                    task.priority.label(),
                                    task.progress
                                ),
                            )),
                    )
                    .child(self.small_action_button(
                        "close-center-editor",
                        "閉じる",
                        cx,
                        |this, _, cx| {
                            this.flush_pending_edits(cx);
                            this.selected_task = None;
                            this.show_task_editor_details = false;
                            cx.notify();
                        },
                    )),
            )
            .child(
                div()
                    .flex()
                    .items_end()
                    .gap_3()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .gap_1()
                            .child(
                                div()
                                    .text_size(px(12.0))
                                    .text_color(theme::MUTED)
                                    .child("タイトル"),
                            )
                            .child(Input::new(&self.title_input)),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .w(px(240.0))
                            .gap_1()
                            .child(
                                div()
                                    .text_size(px(12.0))
                                    .text_color(theme::MUTED)
                                    .child("納期"),
                            )
                            .child(Input::new(&self.due_input)),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(theme::MUTED)
                            .child("メモ"),
                    )
                    .child(Input::new(&self.memo_input).h(px(110.0))),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child({
                        let entity = cx.entity();
                        Button::new("save-center-editor")
                            .primary()
                            .label("変更を保存")
                            .on_click(move |_, _, cx| {
                                entity.update(cx, |this, cx| {
                                    this.save_and_close_selected_task(cx);
                                });
                            })
                    })
                    .child({
                        let entity = cx.entity();
                        Button::new("toggle-center-editor-details")
                            .small()
                            .label(if self.show_task_editor_details {
                                "詳細項目を閉じる"
                            } else {
                                "状態・優先度など"
                            })
                            .selected(self.show_task_editor_details)
                            .on_click(move |_, _, cx| {
                                entity.update(cx, |this, cx| {
                                    this.show_task_editor_details = !this.show_task_editor_details;
                                    cx.notify();
                                });
                            })
                    })
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(theme::MUTED)
                            .child("タイトルとメモは入力停止後にも自動保存されます"),
                    ),
            )
            .when(self.show_task_editor_details, |editor| {
                editor.child(self.render_center_task_details(id, &task, cx))
            })
            .into_any_element()
    }

    fn render_center_task_details(
        &self,
        id: TaskId,
        task: &Task,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .gap_3()
            .pt_2()
            .border_t_1()
            .border_color(theme::BORDER)
            .child(
                div()
                    .flex()
                    .items_center()
                    .flex_wrap()
                    .gap_2()
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(theme::MUTED)
                            .child("状態"),
                    )
                    .children(
                        TaskStatus::ALL
                            .into_iter()
                            .filter(|status| *status != TaskStatus::Archived)
                            .map(|status| {
                                let entity = cx.entity();
                                Button::new(SharedString::from(format!(
                                    "center-status-{}",
                                    status.as_str()
                                )))
                                .small()
                                .label(status.label())
                                .selected(task.status == status)
                                .on_click(move |_, _, cx| {
                                    entity.update(cx, |this, cx| {
                                        this.set_task_status(id, status, cx);
                                    });
                                })
                            }),
                    )
                    .child(
                        div()
                            .ml_3()
                            .text_size(px(12.0))
                            .text_color(theme::MUTED)
                            .child("優先度"),
                    )
                    .children(Priority::ALL.into_iter().map(|priority| {
                        let entity = cx.entity();
                        Button::new(SharedString::from(format!(
                            "center-priority-{}",
                            priority.as_str()
                        )))
                        .small()
                        .label(priority.label())
                        .selected(task.priority == priority)
                        .on_click(move |_, _, cx| {
                            entity.update(cx, |this, cx| {
                                this.set_task_priority(id, priority, cx);
                            });
                        })
                    })),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .flex_wrap()
                    .gap_2()
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(theme::MUTED)
                            .child("進捗"),
                    )
                    .child(Input::new(&self.progress_input).w(px(90.0)))
                    .children([0, 25, 50, 75, 100].into_iter().map(|progress| {
                        let entity = cx.entity();
                        Button::new(SharedString::from(format!("center-progress-{progress}")))
                            .small()
                            .label(format!("{progress}%"))
                            .selected(task.progress == progress)
                            .on_click(move |_, _, cx| {
                                entity.update(cx, |this, cx| {
                                    this.set_task_progress(id, progress, cx);
                                });
                            })
                    }))
                    .child(
                        div()
                            .ml_3()
                            .text_size(px(12.0))
                            .text_color(theme::MUTED)
                            .child("プロジェクト"),
                    )
                    .child({
                        let entity = cx.entity();
                        Button::new("center-project-none")
                            .small()
                            .label("なし")
                            .selected(task.project_id.is_none())
                            .on_click(move |_, _, cx| {
                                entity.update(cx, |this, cx| {
                                    this.set_task_project(id, None, cx);
                                });
                            })
                    })
                    .children(self.projects.iter().map(|project| {
                        let project_id = project.id;
                        let entity = cx.entity();
                        Button::new(SharedString::from(format!("center-project-{project_id}")))
                            .small()
                            .label(project.name.clone())
                            .selected(task.project_id == Some(project_id))
                            .on_click(move |_, _, cx| {
                                entity.update(cx, |this, cx| {
                                    this.set_task_project(id, Some(project_id), cx);
                                });
                            })
                    })),
            )
            .when(!self.tags.is_empty(), |details| {
                details.child(
                    div()
                        .flex()
                        .items_center()
                        .flex_wrap()
                        .gap_2()
                        .child(
                            div()
                                .text_size(px(12.0))
                                .text_color(theme::MUTED)
                                .child("タグ"),
                        )
                        .children(self.tags.iter().map(|tag| {
                            let tag_id = tag.id;
                            let entity = cx.entity();
                            Checkbox::new(SharedString::from(format!("center-tag-{tag_id}")))
                                .label(tag.name.clone())
                                .checked(task.tag_ids.contains(&tag_id))
                                .on_click(move |_, _, cx| {
                                    entity.update(cx, |this, cx| {
                                        this.toggle_task_tag(id, tag_id, cx);
                                    });
                                })
                        })),
                )
            })
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child({
                        let entity = cx.entity();
                        Button::new("center-archive-task")
                            .small()
                            .label("アーカイブ")
                            .on_click(move |_, _, cx| {
                                entity.update(cx, |this, cx| {
                                    this.set_task_status(id, TaskStatus::Archived, cx);
                                });
                            })
                    })
                    .child({
                        let entity = cx.entity();
                        Button::new("center-trash-task")
                            .small()
                            .danger()
                            .label("ゴミ箱へ")
                            .on_click(move |_, _, cx| {
                                entity.update(cx, |this, cx| this.move_to_trash(id, cx));
                            })
                    }),
            )
            .into_any_element()
    }

    fn render_list(&self, tasks: Vec<Task>, cx: &mut Context<Self>) -> AnyElement {
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
        let mut grouped_tasks = tasks;
        if let Some(group) = self.group_by {
            grouped_tasks.sort_by_key(|task| self.task_group_label(task, group));
        }
        for task in grouped_tasks {
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

    fn render_task_row(&self, task: Task, cx: &mut Context<Self>) -> AnyElement {
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
        let priority_color = priority_color(task.priority);
        let project_label = task.project_id.and_then(|id| {
            self.projects
                .iter()
                .find(|project| project.id == id)
                .map(|project| project.name.clone())
        });
        let tag_label = {
            let names = task
                .tag_ids
                .iter()
                .filter_map(|id| self.tags.iter().find(|tag| tag.id == *id))
                .take(2)
                .map(|tag| format!("#{}", tag.name))
                .collect::<Vec<_>>();
            (!names.is_empty()).then(|| names.join(" "))
        };
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
            .flex()
            .items_center()
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
                    .flex()
                    .flex_col()
                    .gap_1()
                    .flex_1()
                    .child(
                        div()
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
                            .items_center()
                            .gap_3()
                            .text_size(px(12.0))
                            .text_color(theme::MUTED)
                            .child(if task.status == TaskStatus::Blocked {
                                "⏸ 保留"
                            } else {
                                task.status.label()
                            })
                            .child(
                                div()
                                    .text_color(priority_color)
                                    .child(format!("優先度 {}", task.priority.label())),
                            )
                            .when(!due.is_empty(), |line| {
                                line.child(div().text_color(due_color).child(due))
                            })
                            .when_some(project_label, |line, project| {
                                line.child(format!("プロジェクト {project}"))
                            })
                            .when_some(tag_label, |line, tags| line.child(tags)),
                    )
                    .child(Progress::new().value(f32::from(task.progress))),
            )
            .child(format!("{}%", task.progress))
            .child({
                let entity = cx.entity();
                Button::new(SharedString::from(format!("move-up-{task_id}")))
                    .ghost()
                    .small()
                    .label("↑")
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
                        .label("受信箱へ")
                        .on_click(move |_, _, cx| {
                            entity.update(cx, |this, cx| {
                                this.set_task_status(task_id, TaskStatus::Inbox, cx);
                            });
                        }),
                )
            })
            .child(
                Button::new(SharedString::from(format!("delete-{task_id}")))
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
                        "受信箱へ戻す"
                    } else {
                        "アーカイブ"
                    })
                    .on_click(move |_, _, cx| {
                        archive_entity.update(cx, |this, cx| {
                            this.set_task_status(
                                task_id,
                                if task.status == TaskStatus::Archived {
                                    TaskStatus::Inbox
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
        let leading = first.weekday().number_days_from_monday() as usize;
        let days = first.month().length(first.year()) as usize;
        let mut cells = Vec::with_capacity(42);
        for cell in 0..42 {
            if cell < leading || cell >= leading + days {
                cells.push(
                    div()
                        .h(px(104.0))
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
                    .h(px(104.0))
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
        let weekdays = ["月", "火", "水", "木", "金", "土", "日"];
        let calendar = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .overflow_y_scrollbar()
            .child(
                div()
                    .grid()
                    .grid_cols(7)
                    .children(weekdays.into_iter().enumerate().map(|(index, day)| {
                        div()
                            .py_1()
                            .text_center()
                            .text_color(match index {
                                5 => theme::ACCENT,
                                6 => theme::DANGER,
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
            .min_h_0()
            .gap_3()
            .p_4()
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

    fn render_detail(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(task) = self.selected_task().cloned() else {
            match self.active_view {
                SmartView::Project(id) => return self.render_project_detail(id, cx),
                SmartView::Tag(id) => return self.render_tag_detail(id, cx),
                _ => {}
            }
            return div()
                .flex()
                .items_center()
                .justify_center()
                .w(px(self.settings.detail_width))
                .h_full()
                .flex_shrink_0()
                .border_l_1()
                .border_color(theme::BORDER)
                .bg(theme::SURFACE)
                .text_color(theme::MUTED)
                .child("タスクを選択してください")
                .into_any_element();
        };
        let id = task.id;
        div()
            .flex()
            .flex_col()
            .w(px(self.settings.detail_width))
            .h_full()
            .flex_shrink_0()
            .border_l_1()
            .border_color(theme::BORDER)
            .bg(theme::SURFACE)
            .overflow_y_scrollbar()
            .p_4()
            .gap_4()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_size(px(18.0))
                            .font_weight(FontWeight::BOLD)
                            .child("タスク詳細"),
                    )
                    .child(self.small_action_button(
                        "close-detail",
                        "閉じる",
                        cx,
                        |this, _, cx| {
                            this.selected_task = None;
                            cx.notify();
                        },
                    )),
            )
            .child(labeled_input("タイトル", Input::new(&self.title_input)))
            .child(labeled_input(
                "メモ",
                Input::new(&self.memo_input).h(px(160.0)),
            ))
            .child(section_label("状態"))
            .child(
                div().flex().flex_wrap().gap_2().children(
                    TaskStatus::ALL
                        .into_iter()
                        .filter(|status| *status != TaskStatus::Archived)
                        .map(|status| {
                            let entity = cx.entity();
                            Button::new(SharedString::from(format!("status-{}", status.as_str())))
                                .small()
                                .label(status.label())
                                .selected(task.status == status)
                                .on_click(move |_, _, cx| {
                                    entity.update(cx, |this, cx| {
                                        this.set_task_status(id, status, cx);
                                    });
                                })
                        }),
                ),
            )
            .child(section_label("優先度"))
            .child(
                div()
                    .flex()
                    .gap_2()
                    .children(Priority::ALL.into_iter().map(|priority| {
                        let entity = cx.entity();
                        Button::new(SharedString::from(format!(
                            "priority-{}",
                            priority.as_str()
                        )))
                        .small()
                        .label(priority.label())
                        .selected(task.priority == priority)
                        .on_click(move |_, _, cx| {
                            entity.update(cx, |this, cx| {
                                this.set_task_priority(id, priority, cx);
                            });
                        })
                    })),
            )
            .child(section_label("進捗"))
            .child(Progress::new().value(f32::from(task.progress)))
            .child(labeled_input(
                "直接入力（0〜100）",
                Input::new(&self.progress_input),
            ))
            .child(
                div()
                    .flex()
                    .gap_2()
                    .children([0, 25, 50, 75, 100].into_iter().map(|progress| {
                        let entity = cx.entity();
                        Button::new(SharedString::from(format!("progress-{progress}")))
                            .small()
                            .label(format!("{progress}%"))
                            .selected(task.progress == progress)
                            .on_click(move |_, _, cx| {
                                entity.update(cx, |this, cx| {
                                    this.set_task_progress(id, progress, cx);
                                });
                            })
                    })),
            )
            .child(labeled_input("納期", Input::new(&self.due_input)))
            .child(
                div().text_size(px(12.0)).text_color(theme::MUTED).child(
                    "空欄、日付、日時を指定できます。Enterまたは下部の保存ボタンで確定します。",
                ),
            )
            .child(section_label("プロジェクト"))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .child({
                        let entity = cx.entity();
                        Button::new("project-none")
                            .small()
                            .label("なし")
                            .selected(task.project_id.is_none())
                            .on_click(move |_, _, cx| {
                                entity.update(cx, |this, cx| {
                                    this.set_task_project(id, None, cx);
                                });
                            })
                    })
                    .children(self.projects.iter().map(|project| {
                        let project_id = project.id;
                        let entity = cx.entity();
                        Button::new(SharedString::from(format!("project-{project_id}")))
                            .small()
                            .label(project.name.clone())
                            .selected(task.project_id == Some(project_id))
                            .on_click(move |_, _, cx| {
                                entity.update(cx, |this, cx| {
                                    this.set_task_project(id, Some(project_id), cx);
                                });
                            })
                    })),
            )
            .child(section_label("タグ"))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .children(self.tags.iter().map(|tag| {
                        let tag_id = tag.id;
                        let entity = cx.entity();
                        Checkbox::new(SharedString::from(format!("tag-{tag_id}")))
                            .label(tag.name.clone())
                            .checked(task.tag_ids.contains(&tag_id))
                            .on_click(move |_, _, cx| {
                                entity.update(cx, |this, cx| {
                                    this.toggle_task_tag(id, tag_id, cx);
                                });
                            })
                    })),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .mt_4()
                    .child({
                        let entity = cx.entity();
                        Button::new("save-task-detail")
                            .primary()
                            .label("変更を保存")
                            .on_click(move |_, _, cx| {
                                entity.update(cx, |this, cx| {
                                    this.save_selected_task_form(cx);
                                });
                            })
                    })
                    .child(div().text_size(px(12.0)).text_color(theme::MUTED).child(
                        "タイトルとメモは自動保存されますが、このボタンでも明示的に保存できます。",
                    )),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child({
                        let entity = cx.entity();
                        Button::new("archive-task")
                            .label("アーカイブ")
                            .on_click(move |_, _, cx| {
                                entity.update(cx, |this, cx| {
                                    this.set_task_status(id, TaskStatus::Archived, cx);
                                });
                            })
                    })
                    .child({
                        let entity = cx.entity();
                        Button::new("trash-task")
                            .danger()
                            .label("ゴミ箱へ")
                            .on_click(move |_, _, cx| {
                                entity.update(cx, |this, cx| {
                                    this.move_to_trash(id, cx);
                                });
                            })
                    }),
            )
            .into_any_element()
    }

    fn render_project_detail(&self, id: ProjectId, cx: &mut Context<Self>) -> AnyElement {
        let Some(project) = self.projects.iter().find(|project| project.id == id) else {
            return div().into_any_element();
        };
        div()
            .flex()
            .flex_col()
            .w(px(self.settings.detail_width))
            .h_full()
            .flex_shrink_0()
            .border_l_1()
            .border_color(theme::BORDER)
            .bg(theme::SURFACE)
            .overflow_y_scrollbar()
            .p_4()
            .gap_4()
            .child(
                div()
                    .text_size(px(18.0))
                    .font_weight(FontWeight::BOLD)
                    .child("プロジェクト詳細"),
            )
            .child(
                div()
                    .text_size(px(12.0))
                    .text_color(theme::MUTED)
                    .child(self.project_summary(project)),
            )
            .child(labeled_input("名前", Input::new(&self.project_input)))
            .child(labeled_input(
                "説明",
                Input::new(&self.project_description_input).h(px(120.0)),
            ))
            .child(labeled_input("色", Input::new(&self.project_color_input)))
            .child(self.small_action_button(
                "save-project-detail",
                "変更を保存",
                cx,
                |this, window, cx| this.update_active_project(window, cx),
            ))
            .child({
                let entity = cx.entity();
                Button::new("archive-project-detail")
                    .label(if project.archived_at.is_some() {
                        "アーカイブから戻す"
                    } else {
                        "アーカイブ"
                    })
                    .on_click(move |_, _, cx| {
                        entity.update(cx, |this, cx| this.toggle_project_archive(id, cx));
                    })
            })
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .child({
                        let entity = cx.entity();
                        Button::new("project-up-detail")
                            .small()
                            .label("上へ移動")
                            .on_click(move |_, _, cx| {
                                entity.update(cx, |this, cx| {
                                    this.move_project_order(id, -1, cx);
                                });
                            })
                    })
                    .child({
                        let entity = cx.entity();
                        Button::new("project-down-detail")
                            .small()
                            .label("下へ移動")
                            .on_click(move |_, _, cx| {
                                entity.update(cx, |this, cx| {
                                    this.move_project_order(id, 1, cx);
                                });
                            })
                    })
                    .child({
                        let entity = cx.entity();
                        Button::new("delete-project-detail")
                            .small()
                            .danger()
                            .label("プロジェクトを削除")
                            .on_click(move |_, _, cx| {
                                entity.update(cx, |this, cx| this.delete_project(id, cx));
                            })
                    }),
            )
            .into_any_element()
    }

    fn render_tag_detail(&self, id: TagId, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .w(px(self.settings.detail_width))
            .h_full()
            .flex_shrink_0()
            .border_l_1()
            .border_color(theme::BORDER)
            .bg(theme::SURFACE)
            .p_4()
            .gap_4()
            .child(
                div()
                    .text_size(px(18.0))
                    .font_weight(FontWeight::BOLD)
                    .child("タグ詳細"),
            )
            .child(labeled_input("名前", Input::new(&self.tag_input)))
            .child(labeled_input("色", Input::new(&self.tag_color_input)))
            .child(self.small_action_button(
                "save-tag-detail",
                "変更を保存",
                cx,
                |this, window, cx| this.update_active_tag(window, cx),
            ))
            .child(
                div()
                    .text_size(px(12.0))
                    .text_color(theme::MUTED)
                    .child(format!(
                        "{}件のタスクで使用中",
                        self.tasks
                            .iter()
                            .filter(|task| task.tag_ids.contains(&id) && task.deleted_at.is_none())
                            .count()
                    )),
            )
            .child({
                let entity = cx.entity();
                Button::new("delete-tag-detail")
                    .small()
                    .danger()
                    .label("タグを削除")
                    .on_click(move |_, _, cx| {
                        entity.update(cx, |this, cx| this.delete_tag(id, cx));
                    })
            })
            .into_any_element()
    }

    fn active_view_label(&self) -> String {
        match self.active_view {
            SmartView::Inbox => "受信箱".to_owned(),
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
            SmartView::Project(id) => self
                .projects
                .iter()
                .find(|project| project.id == id)
                .map(|project| project.name.clone())
                .unwrap_or_else(|| "プロジェクト".to_owned()),
            SmartView::Tag(id) => self
                .tags
                .iter()
                .find(|tag| tag.id == id)
                .map(|tag| format!("#{}", tag.name))
                .unwrap_or_else(|| "タグ".to_owned()),
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
        let show_management_detail = self.selected_task.is_none()
            && matches!(self.active_view, SmartView::Project(_) | SmartView::Tag(_));
        let resize_workspace = cx.entity();
        let main_panes = h_resizable("workspace-main-panes")
            .with_state(&self.sidebar_resize_state)
            .on_resize(move |state, _, cx| {
                let Some(width) = state.read(cx).sizes().first().copied() else {
                    return;
                };
                resize_workspace.update(cx, |this, cx| {
                    this.settings.sidebar_width = f32::from(width).clamp(180.0, 380.0);
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
                    .size_range(px(500.0)..gpui::Pixels::MAX)
                    .child(self.render_content(cx)),
            );
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
                this.new_task_input.read(cx).focus_handle(cx).focus(window);
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
                this.selected_task = None;
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
                    .relative()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .child(main_panes)
                    .when(show_management_detail, |content| {
                        content.child(
                            div()
                                .absolute()
                                .right_0()
                                .top_0()
                                .bottom_0()
                                .shadow_lg()
                                .child(self.render_detail(cx)),
                        )
                    }),
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
    if let Some(id) = value
        .strip_prefix("project:")
        .and_then(|id| id.parse().ok())
    {
        return SmartView::Project(id);
    }
    if let Some(id) = value.strip_prefix("tag:").and_then(|id| id.parse().ok()) {
        return SmartView::Tag(id);
    }
    if let Some(id) = value.strip_prefix("saved:").and_then(|id| id.parse().ok()) {
        return SmartView::Saved(id);
    }
    match value {
        "inbox" => SmartView::Inbox,
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
        SmartView::Inbox => "inbox",
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
        SmartView::Project(id) => return format!("project:{id}"),
        SmartView::Tag(id) => return format!("tag:{id}"),
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

fn parse_due(value: &str) -> Result<Due, String> {
    let value = value.trim();
    if value.is_empty() || value == "未定" {
        return Ok(Due::None);
    }
    let date_format = format_description!("[year]-[month]-[day]");
    if let Ok(date) = Date::parse(value, date_format) {
        return Ok(Due::Date(date));
    }
    let date_time_format = format_description!("[year]-[month]-[day] [hour]:[minute]");
    if let Ok(date_time) = PrimitiveDateTime::parse(value, date_time_format) {
        let offset = UtcOffset::current_local_offset().unwrap_or(UtcOffset::UTC);
        return Ok(Due::DateTime(date_time.assume_offset(offset)));
    }
    Err("納期は YYYY-MM-DD または YYYY-MM-DD HH:MM で入力してください".to_owned())
}

fn format_due_input(due: &Due) -> String {
    match due {
        Due::None => String::new(),
        Due::Date(date) => date.to_string(),
        Due::DateTime(date_time) => {
            let offset = UtcOffset::current_local_offset().unwrap_or(UtcOffset::UTC);
            date_time
                .to_offset(offset)
                .format(format_description!("[year]-[month]-[day] [hour]:[minute]"))
                .unwrap_or_default()
        }
    }
}

fn format_due_display(due: &Due) -> String {
    match due {
        Due::None => String::new(),
        Due::Date(date) => format!("期限 {date}"),
        Due::DateTime(_) => format!("期限 {}", format_due_input(due)),
    }
}

fn due_is_today(due: &Due, today: Date, offset: UtcOffset) -> bool {
    match due {
        Due::None => false,
        Due::Date(date) => *date == today,
        Due::DateTime(date_time) => date_time.to_offset(offset).date() == today,
    }
}

fn due_date(due: &Due) -> Option<Date> {
    match due {
        Due::None => None,
        Due::Date(date) => Some(*date),
        Due::DateTime(date_time) => Some(
            date_time
                .to_offset(UtcOffset::current_local_offset().unwrap_or(UtcOffset::UTC))
                .date(),
        ),
    }
}

fn date_to_filter_datetime(date: Date) -> OffsetDateTime {
    date.with_hms(0, 0, 0)
        .expect("midnight must be valid")
        .assume_offset(UtcOffset::current_local_offset().unwrap_or(UtcOffset::UTC))
}

fn shift_month(date: Date, delta: i32) -> Date {
    let mut year = date.year();
    let mut month = i32::from(date.month() as u8) + delta;
    while month < 1 {
        month += 12;
        year -= 1;
    }
    while month > 12 {
        month -= 12;
        year += 1;
    }
    let month = time::Month::try_from(month as u8).expect("month must be valid");
    Date::from_calendar_date(year, month, 1).expect("first day of month must be valid")
}

fn due_is_upcoming(due: &Due, today: Date, offset: UtcOffset) -> bool {
    let end = today + time::Duration::days(7);
    let date = match due {
        Due::None => return false,
        Due::Date(date) => *date,
        Due::DateTime(date_time) => date_time.to_offset(offset).date(),
    };
    date >= today && date <= end
}

fn task_matches_saved_view(task: &Task, view: &SavedView) -> bool {
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
        Some(SavedBaseView::Inbox) => task.status == TaskStatus::Inbox,
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
        && (filter.statuses.is_empty() || filter.statuses.contains(&task.status))
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

fn compare_tasks(left: &Task, right: &Task, sort: &[SortSpec]) -> Ordering {
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

fn unix_millis() -> i128 {
    OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000
}

fn is_hex_color(value: &str) -> bool {
    value.len() == 7
        && value.starts_with('#')
        && value[1..].bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn parse_hex_color(value: &str) -> Option<Rgba> {
    if !is_hex_color(value) {
        return None;
    }
    let red = u8::from_str_radix(&value[1..3], 16).ok()?;
    let green = u8::from_str_radix(&value[3..5], 16).ok()?;
    let blue = u8::from_str_radix(&value[5..7], 16).ok()?;
    Some(Rgba {
        r: f32::from(red) / 255.0,
        g: f32::from(green) / 255.0,
        b: f32::from(blue) / 255.0,
        a: 1.0,
    })
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
                assert_eq!(workspace.active_view, SmartView::Inbox);
                assert_eq!(workspace.view_kind, ViewKind::List);
                workspace.select_task(task_id, window, cx);
                workspace.show_task_editor_details = true;
                workspace.view_kind = ViewKind::Calendar;
                let _selected_calendar_tree = workspace.render(window, cx).into_any_element();
                assert!(workspace.save_and_close_selected_task(cx));
                assert!(workspace.selected_task.is_none());

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
    fn ten_thousand_task_visible_search_completes_within_100ms() {
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
        matches.sort_by(|left, right| compare_tasks(left, right, &[SortSpec::default()]));

        assert_eq!(matches.len(), 1);
        assert!(
            started.elapsed() < StdDuration::from_millis(100),
            "10,000 task visible search took {:?}",
            started.elapsed()
        );
    }
}
