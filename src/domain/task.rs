use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use time::{Date, OffsetDateTime};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TaskId(Uuid);

impl TaskId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for TaskId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for TaskId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl FromStr for TaskId {
    type Err = uuid::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Uuid::parse_str(value).map(Self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Inbox,
    Todo,
    Doing,
    Blocked,
    Done,
    Archived,
}

impl TaskStatus {
    pub const ALL: [Self; 6] = [
        Self::Inbox,
        Self::Todo,
        Self::Doing,
        Self::Blocked,
        Self::Done,
        Self::Archived,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Inbox => "inbox",
            Self::Todo => "todo",
            Self::Doing => "doing",
            Self::Blocked => "blocked",
            Self::Done => "done",
            Self::Archived => "archived",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Inbox => "受信箱",
            Self::Todo => "未着手",
            Self::Doing => "進行中",
            Self::Blocked => "保留",
            Self::Done => "完了",
            Self::Archived => "アーカイブ",
        }
    }
}

impl FromStr for TaskStatus {
    type Err = TaskError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "inbox" => Ok(Self::Inbox),
            "todo" => Ok(Self::Todo),
            "doing" => Ok(Self::Doing),
            "blocked" => Ok(Self::Blocked),
            "done" => Ok(Self::Done),
            "archived" => Ok(Self::Archived),
            other => Err(TaskError::InvalidStatus(other.to_owned())),
        }
    }
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default,
)]
#[serde(rename_all = "snake_case")]
pub enum Priority {
    #[default]
    None,
    Low,
    Medium,
    High,
}

impl Priority {
    pub const ALL: [Self; 4] = [Self::None, Self::Low, Self::Medium, Self::High];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::None => "なし",
            Self::Low => "低",
            Self::Medium => "中",
            Self::High => "高",
        }
    }
}

impl FromStr for Priority {
    type Err = TaskError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "none" => Ok(Self::None),
            "low" => Ok(Self::Low),
            "medium" => Ok(Self::Medium),
            "high" => Ok(Self::High),
            other => Err(TaskError::InvalidPriority(other.to_owned())),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum Due {
    #[default]
    None,
    Date(Date),
    DateTime(OffsetDateTime),
}

impl Due {
    pub fn is_overdue(&self, now: OffsetDateTime, today: Date) -> bool {
        match self {
            Self::None => false,
            Self::Date(date) => *date < today,
            Self::DateTime(date_time) => *date_time < now,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Task {
    pub id: TaskId,
    pub title: String,
    pub memo: String,
    pub status: TaskStatus,
    pub priority: Priority,
    pub progress: u8,
    pub due: Due,
    pub project_id: Option<super::ProjectId>,
    pub tag_ids: Vec<super::TagId>,
    pub sort_order: i64,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
    pub completed_at: Option<OffsetDateTime>,
    pub deleted_at: Option<OffsetDateTime>,
}

impl Task {
    pub fn new(title: impl Into<String>, now: OffsetDateTime) -> Result<Self, TaskError> {
        let title = normalize_title(title.into())?;
        Ok(Self {
            id: TaskId::new(),
            title,
            memo: String::new(),
            status: TaskStatus::Inbox,
            priority: Priority::None,
            progress: 0,
            due: Due::None,
            project_id: None,
            tag_ids: Vec::new(),
            sort_order: 0,
            created_at: now,
            updated_at: now,
            completed_at: None,
            deleted_at: None,
        })
    }

    pub fn set_title(&mut self, title: impl Into<String>) -> Result<(), TaskError> {
        self.title = normalize_title(title.into())?;
        Ok(())
    }

    pub fn set_progress(&mut self, progress: u8) -> Result<(), TaskError> {
        if progress > 100 {
            return Err(TaskError::InvalidProgress(progress));
        }
        self.progress = progress;
        Ok(())
    }

    pub fn set_status(&mut self, status: TaskStatus, now: OffsetDateTime) {
        self.status = status;
        self.updated_at = now;
        if status == TaskStatus::Done {
            self.progress = 100;
            self.completed_at = Some(now);
        } else {
            if self.completed_at.is_some() {
                self.progress = 0;
            }
            self.completed_at = None;
        }
    }

    pub fn touch(&mut self, now: OffsetDateTime) {
        self.updated_at = now;
    }

    pub fn move_to_trash(&mut self, now: OffsetDateTime) {
        self.deleted_at = Some(now);
        self.updated_at = now;
    }

    pub fn restore(&mut self, now: OffsetDateTime) {
        self.deleted_at = None;
        self.updated_at = now;
    }
}

fn normalize_title(title: String) -> Result<String, TaskError> {
    let title = title.trim();
    let length = title.chars().count();
    if length == 0 {
        return Err(TaskError::EmptyTitle);
    }
    if length > 500 {
        return Err(TaskError::TitleTooLong(length));
    }
    Ok(title.to_owned())
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum TaskError {
    #[error("タイトルを入力してください")]
    EmptyTitle,
    #[error("タイトルは500文字以内で入力してください（現在: {0}文字）")]
    TitleTooLong(usize),
    #[error("進捗率は0から100の範囲で指定してください: {0}")]
    InvalidProgress(u8),
    #[error("不明な状態です: {0}")]
    InvalidStatus(String),
    #[error("不明な優先度です: {0}")]
    InvalidPriority(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completing_and_reopening_updates_progress_and_timestamp() {
        let now = OffsetDateTime::UNIX_EPOCH;
        let mut task = Task::new("test", now).unwrap();
        task.set_progress(40).unwrap();
        task.set_status(TaskStatus::Done, now);
        assert_eq!(task.progress, 100);
        assert_eq!(task.completed_at, Some(now));

        task.set_status(TaskStatus::Doing, now);
        assert_eq!(task.progress, 0);
        assert_eq!(task.completed_at, None);
    }

    #[test]
    fn title_is_trimmed_and_validated() {
        let now = OffsetDateTime::UNIX_EPOCH;
        let task = Task::new("  example  ", now).unwrap();
        assert_eq!(task.title, "example");
        assert_eq!(Task::new("   ", now), Err(TaskError::EmptyTitle));
    }
}
