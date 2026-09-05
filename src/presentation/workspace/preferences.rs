//! Persist presentation preferences independently of unsaved task edits.
use gpui::Context;

use crate::domain::ViewKind;

use super::{Workspace, smart_view_setting};

const SAVE_ERROR_PREFIX: &str = "表示設定の保存に失敗しました: ";

impl Workspace {
    pub(super) fn set_view_kind(&mut self, kind: ViewKind, cx: &mut Context<Self>) {
        self.view_kind = kind;
        self.persist_view_preferences(cx);
        cx.notify();
    }

    pub(super) fn persist_view_preferences(&mut self, cx: &mut Context<Self>) {
        self.update_persisted_settings_fields();
        match self.settings.save(&self.paths.settings) {
            Err(error) => {
                self.error_message = Some(format!("{SAVE_ERROR_PREFIX}{error}"));
                cx.notify();
            }
            Ok(()) => {
                // Only dismiss our own recovered error, not an unrelated task error.
                if self
                    .error_message
                    .as_deref()
                    .is_some_and(|message| message.starts_with(SAVE_ERROR_PREFIX))
                {
                    self.error_message = None;
                    cx.notify();
                }
            }
        }
    }

    pub(super) fn update_persisted_settings_fields(&mut self) {
        self.settings.view_kind = self.view_kind;
        self.settings.active_view = smart_view_setting(self.active_view);
        self.settings.sort = self.sort.clone();
        self.settings.group_by = self.group_by;
    }
}

#[cfg(test)]
mod tests;
