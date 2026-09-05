//! Persist presentation preferences independently of unsaved task edits.
use gpui::Context;

use crate::domain::ViewKind;

use super::{Workspace, smart_view_setting};

impl Workspace {
    pub(super) fn set_view_kind(&mut self, kind: ViewKind, cx: &mut Context<Self>) {
        self.view_kind = kind;
        self.persist_view_preferences(cx);
        cx.notify();
    }

    pub(super) fn persist_view_preferences(&mut self, cx: &mut Context<Self>) {
        self.update_persisted_settings_fields();
        if let Err(error) = self.settings.save(&self.paths.settings) {
            self.error_message = Some(format!("表示設定の保存に失敗しました: {error}"));
            cx.notify();
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
