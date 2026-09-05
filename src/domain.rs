mod project;
mod saved_view;
mod tag;
mod task;

pub use project::{Project, ProjectId};
pub use saved_view::{
    DueScope, GroupBy, SavedBaseView, SavedView, SavedViewId, SortDirection, SortField, SortSpec,
    TaskFilter, ViewKind,
};
pub use tag::{Tag, TagId};
pub use task::{Due, Priority, Task, TaskError, TaskId, TaskStatus};

pub(crate) mod task_query;

/// Persisted entities loaded together for one workspace.
#[derive(Debug, Clone)]
pub struct AppDataSnapshot {
    pub tasks: Vec<Task>,
    pub projects: Vec<Project>,
    pub tags: Vec<Tag>,
    pub saved_views: Vec<SavedView>,
}
