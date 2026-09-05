//! Applying and persisting reversible changes without a dependency on GPUI.
use super::{ApplicationError, TaskApplication};
use crate::domain::{Project, Tag, Task};

/// One reversible application-level operation.
///
/// A single entry may span tasks and their related project or tag so that
/// relationship changes are restored together.
#[derive(Debug, Clone)]
pub(crate) struct HistoryEntry {
    pub(crate) task_changes: Vec<(Option<Task>, Option<Task>)>,
    pub(crate) project_changes: Vec<(Option<Project>, Option<Project>)>,
    pub(crate) tag_changes: Vec<(Option<Tag>, Option<Tag>)>,
}

impl HistoryEntry {
    pub(crate) fn tasks(changes: Vec<(Option<Task>, Option<Task>)>) -> Self {
        Self {
            task_changes: changes,
            project_changes: Vec::new(),
            tag_changes: Vec::new(),
        }
    }

    pub(crate) fn apply(
        &self,
        tasks: &mut Vec<Task>,
        projects: &mut Vec<Project>,
        tags: &mut Vec<Tag>,
        use_after: bool,
    ) {
        for (before, after) in &self.task_changes {
            let state = if use_after { after } else { before };
            let id = before
                .as_ref()
                .or(after.as_ref())
                .map(|task| task.id)
                .expect("task history entry must have a task");
            match state {
                Some(task) => {
                    if let Some(existing) = tasks.iter_mut().find(|item| item.id == id) {
                        *existing = task.clone();
                    } else {
                        tasks.push(task.clone());
                    }
                }
                None => tasks.retain(|task| task.id != id),
            }
        }
        for (before, after) in &self.project_changes {
            let state = if use_after { after } else { before };
            let id = before
                .as_ref()
                .or(after.as_ref())
                .map(|project| project.id)
                .expect("project history entry must have a project");
            match state {
                Some(project) => {
                    if let Some(existing) = projects.iter_mut().find(|item| item.id == id) {
                        *existing = project.clone();
                    } else {
                        projects.push(project.clone());
                    }
                }
                None => projects.retain(|project| project.id != id),
            }
        }
        for (before, after) in &self.tag_changes {
            let state = if use_after { after } else { before };
            let id = before
                .as_ref()
                .or(after.as_ref())
                .map(|tag| tag.id)
                .expect("tag history entry must have a tag");
            match state {
                Some(tag) => {
                    if let Some(existing) = tags.iter_mut().find(|item| item.id == id) {
                        *existing = tag.clone();
                    } else {
                        tags.push(tag.clone());
                    }
                }
                None => tags.retain(|tag| tag.id != id),
            }
        }
    }

    pub(crate) fn persist(
        &self,
        application: &TaskApplication,
        tasks: &[Task],
        use_after: bool,
    ) -> Result<(), ApplicationError> {
        let (projects_to_save, projects_to_delete) = self.project_changes.iter().fold(
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
        let (tags_to_save, tags_to_delete) = self.tag_changes.iter().fold(
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
        application.apply_history_state(
            (!self.task_changes.is_empty()).then(|| tasks.to_vec()),
            projects_to_save,
            projects_to_delete,
            tags_to_save,
            tags_to_delete,
        )
    }
}
