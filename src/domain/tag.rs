use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TagId(Uuid);

impl TagId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for TagId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for TagId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl FromStr for TagId {
    type Err = uuid::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Uuid::parse_str(value).map(Self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tag {
    pub id: TagId,
    pub name: String,
    pub color: Option<String>,
    pub created_at: OffsetDateTime,
}

impl Tag {
    pub fn new(name: impl Into<String>, now: OffsetDateTime) -> Self {
        Self {
            id: TagId::new(),
            name: name.into().trim().to_owned(),
            color: None,
            created_at: now,
        }
    }
}
