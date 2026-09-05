//! SQLite encoding/decoding. Conversion failures retain their column index.
use super::RepositoryError;
use crate::domain::{Due, Priority, ProjectId, Task, TaskStatus};
use rusqlite::types::Type;
use std::str::FromStr;
use thiserror::Error;
use time::{Date, OffsetDateTime, format_description::well_known::Iso8601};

pub(super) fn row_to_task(row: &rusqlite::Row<'_>) -> rusqlite::Result<Task> {
    let due_kind: String = row.get(6)?;
    let due_date: Option<String> = row.get(7)?;
    let due_at: Option<i64> = row.get(8)?;
    let due = match due_kind.as_str() {
        "none" => Due::None,
        "date" => {
            let value = due_date.ok_or_else(|| {
                conversion_error(7, StringError("date due value is missing".to_owned()))
            })?;
            Due::Date(
                Date::parse(&value, &Iso8601::DATE).map_err(|error| conversion_error(7, error))?,
            )
        }
        "datetime" => Due::DateTime(parse_timestamp(
            due_at.ok_or_else(|| {
                conversion_error(8, StringError("datetime due value is missing".to_owned()))
            })?,
            8,
        )?),
        other => {
            return Err(conversion_error(
                6,
                StringError(format!("unknown due kind: {other}")),
            ));
        }
    };
    let project_id = row
        .get::<_, Option<String>>(9)?
        .map(|value| ProjectId::from_str(&value))
        .transpose()
        .map_err(|error| conversion_error(9, error))?;
    Ok(Task {
        id: parse_id(row.get::<_, String>(0)?, 0)?,
        title: row.get(1)?,
        memo: row.get(2)?,
        status: TaskStatus::from_str(&row.get::<_, String>(3)?)
            .map_err(|error| conversion_error(3, error))?,
        priority: Priority::from_str(&row.get::<_, String>(4)?)
            .map_err(|error| conversion_error(4, error))?,
        progress: row.get(5)?,
        due,
        project_id,
        tag_ids: Vec::new(),
        sort_order: row.get(10)?,
        created_at: parse_timestamp(row.get(11)?, 11)?,
        updated_at: parse_timestamp(row.get(12)?, 12)?,
        completed_at: parse_optional_timestamp(row.get(13)?, 13)?,
        deleted_at: parse_optional_timestamp(row.get(14)?, 14)?,
    })
}

pub(super) fn due_columns(
    due: &Due,
) -> Result<(&'static str, Option<String>, Option<i64>), RepositoryError> {
    match due {
        Due::None => Ok(("none", None, None)),
        Due::Date(date) => Ok(("date", Some(date.format(&Iso8601::DATE)?), None)),
        Due::DateTime(date_time) => Ok(("datetime", None, Some(timestamp_millis(*date_time)))),
    }
}

pub(super) fn timestamp_millis(value: OffsetDateTime) -> i64 {
    (value.unix_timestamp_nanos() / 1_000_000) as i64
}

pub(super) fn parse_timestamp(value: i64, index: usize) -> rusqlite::Result<OffsetDateTime> {
    OffsetDateTime::from_unix_timestamp_nanos(i128::from(value) * 1_000_000)
        .map_err(|error| conversion_error(index, error))
}

pub(super) fn parse_optional_timestamp(
    value: Option<i64>,
    index: usize,
) -> rusqlite::Result<Option<OffsetDateTime>> {
    value.map(|value| parse_timestamp(value, index)).transpose()
}

pub(super) fn parse_id<T>(value: String, index: usize) -> rusqlite::Result<T>
where
    T: FromStr,
    T::Err: std::error::Error + Send + Sync + 'static,
{
    T::from_str(&value).map_err(|error| conversion_error(index, error))
}

pub(super) fn conversion_error(
    index: usize,
    error: impl std::error::Error + Send + Sync + 'static,
) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(index, Type::Text, Box::new(error))
}

#[derive(Debug, Error)]
#[error("{0}")]
pub(super) struct StringError(pub(super) String);
