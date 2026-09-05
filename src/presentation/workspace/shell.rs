//! Workspace layout, toolbar and sidebar navigation.
use super::{
    CloseDetailAction, CommandPaletteAction, DeleteAction, MoveDownAction, MoveUpAction,
    NewTaskAction, RedoAction, SearchAction, SelectAllAction, ToggleDoneAction, UndoAction,
};
use gpui::{
    AnyElement, Context, Focusable as _, FontWeight, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, SharedString, Styled as _, Window, div,
    prelude::FluentBuilder as _, px,
};
use gpui_component::{
    Selectable as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    input::Input,
    resizable::{h_resizable, resizable_panel},
    scroll::ScrollableElement as _,
};
use time::{OffsetDateTime, UtcOffset};

use crate::domain::{TaskStatus, ViewKind};

use super::theme;

use super::due::due_is_today;
use super::task_query::saved_view_is_available;
use super::{SmartView, Workspace, section_label};

impl Workspace {
    pub(super) fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
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
                                        this.set_view_kind(kind, cx);
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

    pub(super) fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
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
                    this.activate_view(view, window, cx);
                });
            })
            .into_any_element()
    }

    pub(super) fn active_view_label(&self) -> String {
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

pub(super) fn smart_view_from_setting(value: &str) -> SmartView {
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

pub(super) fn smart_view_setting(view: SmartView) -> String {
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
