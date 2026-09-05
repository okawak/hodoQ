//! UI history stacks and failure rollback; application owns state changes.
use gpui::Context;

use crate::{application::HistoryEntry, domain::Task};

use super::Workspace;

impl Workspace {
    pub(super) fn push_task_history(&mut self, changes: Vec<(Option<Task>, Option<Task>)>) {
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

    pub(super) fn undo(&mut self, cx: &mut Context<Self>) {
        let Some(history) = self.undo_stack.pop() else {
            return;
        };
        history.apply(&mut self.tasks, &mut self.projects, &mut self.tags, false);
        if let Err(error) = history.persist(&self.worker, &self.tasks, false) {
            history.apply(&mut self.tasks, &mut self.projects, &mut self.tags, true);
            self.undo_stack.push(history);
            self.set_error(error);
        } else {
            self.redo_stack.push(history);
            self.status_message = "変更を取り消しました".to_owned();
        }
        cx.notify();
    }

    pub(super) fn redo(&mut self, cx: &mut Context<Self>) {
        let Some(history) = self.redo_stack.pop() else {
            return;
        };
        history.apply(&mut self.tasks, &mut self.projects, &mut self.tags, true);
        if let Err(error) = history.persist(&self.worker, &self.tasks, true) {
            history.apply(&mut self.tasks, &mut self.projects, &mut self.tags, false);
            self.redo_stack.push(history);
            self.set_error(error);
        } else {
            self.undo_stack.push(history);
            self.status_message = "変更をやり直しました".to_owned();
        }
        cx.notify();
    }
}
