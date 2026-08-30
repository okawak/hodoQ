use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

use super::{Priority, ProjectId, TagId, TaskStatus};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SavedViewId(Uuid);

impl SavedViewId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for SavedViewId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for SavedViewId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl FromStr for SavedViewId {
    type Err = uuid::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Uuid::parse_str(value).map(Self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ViewKind {
    #[default]
    List,
    Board,
    Calendar,
}

impl ViewKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::List => "list",
            Self::Board => "board",
            Self::Calendar => "calendar",
        }
    }
}

impl FromStr for ViewKind {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "list" => Ok(Self::List),
            "board" => Ok(Self::Board),
            "calendar" => Ok(Self::Calendar),
            value => Err(format!("unknown view kind: {value}")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DueScope {
    #[default]
    Any,
    Undated,
    Today,
    Upcoming,
    Overdue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum SavedBaseView {
    Inbox,
    Today,
    Upcoming,
    Overdue,
    Undated,
    Doing,
    Blocked,
    Done,
    Archived,
    Trash,
    Project(ProjectId),
    Tag(TagId),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct TaskFilter {
    pub base_view: Option<SavedBaseView>,
    pub query: String,
    pub statuses: Vec<TaskStatus>,
    pub priorities: Vec<Priority>,
    pub project_ids: Vec<ProjectId>,
    pub unassigned_project: bool,
    pub tag_ids: Vec<TagId>,
    pub match_all_tags: bool,
    pub due_from: Option<OffsetDateTime>,
    pub due_to: Option<OffsetDateTime>,
    pub due_scope: DueScope,
    pub include_archived: bool,
    pub only_deleted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SortField {
    Manual,
    Priority,
    Due,
    UpdatedAt,
    CreatedAt,
    Title,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SortDirection {
    Ascending,
    Descending,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SortSpec {
    pub field: SortField,
    pub direction: SortDirection,
}

impl Default for SortSpec {
    fn default() -> Self {
        Self {
            field: SortField::Manual,
            direction: SortDirection::Ascending,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GroupBy {
    Status,
    Project,
    Priority,
    Due,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedView {
    pub id: SavedViewId,
    pub name: String,
    pub view_kind: ViewKind,
    pub filter: TaskFilter,
    pub sort: Vec<SortSpec>,
    pub group_by: Option<GroupBy>,
    pub sort_order: i64,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}
