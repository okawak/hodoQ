//! Backup, export, restore and recovery controls.
use std::{collections::HashSet, fs, path::PathBuf};

use gpui::{
    AnyElement, Context, IntoElement, ParentElement as _, SharedString, Styled as _, Window, div,
    prelude::FluentBuilder as _, px,
};
use gpui_component::{
    Sizable as _,
    button::{Button, ButtonVariants as _},
    input::Input,
};
use time::OffsetDateTime;

use crate::application::TaskApplication;

use super::theme;

use super::{PendingConfirmation, SmartView, Workspace};

impl Workspace {
    pub(super) fn purge_expired_trash_from_ui(&mut self, cx: &mut Context<Self>) {
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

    pub(super) fn create_manual_backup(&mut self, cx: &mut Context<Self>) {
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

    pub(super) fn export_csv(&mut self, current_filter: bool, cx: &mut Context<Self>) {
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

    pub(super) fn export_json(&mut self, cx: &mut Context<Self>) {
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

    pub(super) fn request_restore_path(&mut self, value: &str, cx: &mut Context<Self>) {
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

    pub(super) fn retry_database(&mut self, cx: &mut Context<Self>) {
        match TaskApplication::reconnect(&self.paths.database) {
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

    pub(super) fn render_data_panel(&self, cx: &mut Context<Self>) -> AnyElement {
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

    pub(super) fn render_confirmation(
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
