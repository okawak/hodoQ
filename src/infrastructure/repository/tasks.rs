//! Task SQL and task/tag relationships; each write owns one transaction.
use super::{
    RepositoryError, SqliteRepository,
    mapping::{conversion_error, due_columns, row_to_task, timestamp_millis},
};
use crate::domain::{
    SortSpec, TagId, Task, TaskFilter, TaskId,
    task_query::{TaskQuery, compare_tasks},
};
use rusqlite::{OptionalExtension, Transaction, params};
use std::{collections::HashMap, str::FromStr};
use time::OffsetDateTime;

impl SqliteRepository {
    pub fn save_task(&mut self, task: &Task) -> Result<(), RepositoryError> {
        let transaction = self.connection.transaction()?;
        save_task_in_transaction(&transaction, task)?;
        transaction.commit()?;
        Ok(())
    }

    pub fn save_tasks(&mut self, tasks: &[Task]) -> Result<(), RepositoryError> {
        let transaction = self.connection.transaction()?;
        for task in tasks {
            save_task_in_transaction(&transaction, task)?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn replace_all_tasks(&mut self, tasks: &[Task]) -> Result<(), RepositoryError> {
        let transaction = self.connection.transaction()?;
        transaction.execute("DELETE FROM tasks", [])?;
        for task in tasks {
            save_task_in_transaction(&transaction, task)?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn task(&self, id: TaskId) -> Result<Option<Task>, RepositoryError> {
        let mut statement = self.connection.prepare(
            "SELECT id, title, memo, status, priority, progress, due_kind, due_date, due_at,
                    project_id, sort_order, created_at, updated_at, completed_at, deleted_at
             FROM tasks WHERE id = ?1",
        )?;
        let mut task = statement
            .query_row([id.to_string()], row_to_task)
            .optional()?;
        if let Some(task) = task.as_mut() {
            task.tag_ids = self.tag_ids_for_task(task.id)?;
        }
        Ok(task)
    }

    pub fn list_tasks(
        &self,
        filter: &TaskFilter,
        sort: &[SortSpec],
    ) -> Result<Vec<Task>, RepositoryError> {
        let mut tasks = self.load_all_task_rows()?;
        let offset = time::UtcOffset::current_local_offset().unwrap_or(time::UtcOffset::UTC);
        let query = TaskQuery::new(filter, OffsetDateTime::now_utc(), offset);
        tasks.retain(|task| query.matches(task));
        let default_sort = [SortSpec::default()];
        let sort = if sort.is_empty() { &default_sort } else { sort };
        tasks.sort_by(|left, right| compare_tasks(left, right, sort, offset));
        Ok(tasks)
    }

    pub fn list_all_tasks(&self) -> Result<Vec<Task>, RepositoryError> {
        self.load_all_task_rows()
    }

    fn load_all_task_rows(&self) -> Result<Vec<Task>, RepositoryError> {
        let mut statement = self.connection.prepare(
            "SELECT id, title, memo, status, priority, progress, due_kind, due_date, due_at,
                    project_id, sort_order, created_at, updated_at, completed_at, deleted_at
             FROM tasks",
        )?;
        let rows = statement.query_map([], row_to_task)?;
        let mut tasks = rows.collect::<Result<Vec<_>, _>>()?;
        let mut tag_statement = self
            .connection
            .prepare("SELECT task_id, tag_id FROM task_tags ORDER BY task_id, tag_id")?;
        let tag_rows = tag_statement.query_map([], |row| {
            let task_id = TaskId::from_str(&row.get::<_, String>(0)?)
                .map_err(|error| conversion_error(0, error))?;
            let tag_id = TagId::from_str(&row.get::<_, String>(1)?)
                .map_err(|error| conversion_error(1, error))?;
            Ok((task_id, tag_id))
        })?;
        let mut tags_by_task = HashMap::<TaskId, Vec<TagId>>::new();
        for row in tag_rows {
            let (task_id, tag_id) = row?;
            tags_by_task.entry(task_id).or_default().push(tag_id);
        }
        for task in &mut tasks {
            task.tag_ids = tags_by_task.remove(&task.id).unwrap_or_default();
        }
        Ok(tasks)
    }

    pub fn move_task_to_trash(
        &mut self,
        id: TaskId,
        now: OffsetDateTime,
    ) -> Result<bool, RepositoryError> {
        Ok(self.connection.execute(
            "UPDATE tasks SET deleted_at = ?2, updated_at = ?2 WHERE id = ?1",
            params![id.to_string(), timestamp_millis(now)],
        )? > 0)
    }

    pub fn restore_task(
        &mut self,
        id: TaskId,
        now: OffsetDateTime,
    ) -> Result<bool, RepositoryError> {
        Ok(self.connection.execute(
            "UPDATE tasks SET deleted_at = NULL, updated_at = ?2 WHERE id = ?1",
            params![id.to_string(), timestamp_millis(now)],
        )? > 0)
    }

    pub fn purge_expired_trash(
        &mut self,
        now: OffsetDateTime,
        retention_days: i64,
    ) -> Result<usize, RepositoryError> {
        let cutoff = now - time::Duration::days(retention_days);
        Ok(self.connection.execute(
            "DELETE FROM tasks WHERE deleted_at IS NOT NULL AND deleted_at <= ?1",
            [timestamp_millis(cutoff)],
        )?)
    }

    pub fn empty_trash(&mut self) -> Result<usize, RepositoryError> {
        Ok(self
            .connection
            .execute("DELETE FROM tasks WHERE deleted_at IS NOT NULL", [])?)
    }

    fn tag_ids_for_task(&self, id: TaskId) -> Result<Vec<TagId>, RepositoryError> {
        let mut statement = self
            .connection
            .prepare("SELECT tag_id FROM task_tags WHERE task_id = ?1 ORDER BY tag_id")?;
        statement
            .query_map([id.to_string()], |row| {
                let value: String = row.get(0)?;
                TagId::from_str(&value).map_err(|error| conversion_error(0, error))
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(Into::into)
    }
}

pub(super) fn save_task_in_transaction(
    transaction: &Transaction<'_>,
    task: &Task,
) -> Result<(), RepositoryError> {
    let (due_kind, due_date, due_at) = due_columns(&task.due)?;
    let project_id = task.project_id.map(|id| id.to_string());
    transaction.execute(
        "INSERT INTO tasks
         (id, title, memo, status, priority, progress, due_kind, due_date, due_at,
          project_id, sort_order, created_at, updated_at, completed_at, deleted_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
         ON CONFLICT(id) DO UPDATE SET
           title = excluded.title,
           memo = excluded.memo,
           status = excluded.status,
           priority = excluded.priority,
           progress = excluded.progress,
           due_kind = excluded.due_kind,
           due_date = excluded.due_date,
           due_at = excluded.due_at,
           project_id = excluded.project_id,
           sort_order = excluded.sort_order,
           updated_at = excluded.updated_at,
           completed_at = excluded.completed_at,
           deleted_at = excluded.deleted_at",
        params![
            task.id.to_string(),
            task.title,
            task.memo,
            task.status.as_str(),
            task.priority.as_str(),
            task.progress,
            due_kind,
            due_date,
            due_at,
            project_id,
            task.sort_order,
            timestamp_millis(task.created_at),
            timestamp_millis(task.updated_at),
            task.completed_at.map(timestamp_millis),
            task.deleted_at.map(timestamp_millis),
        ],
    )?;
    transaction.execute(
        "DELETE FROM task_tags WHERE task_id = ?1",
        [task.id.to_string()],
    )?;
    for tag_id in &task.tag_ids {
        transaction.execute(
            "INSERT INTO task_tags (task_id, tag_id) VALUES (?1, ?2)",
            params![task.id.to_string(), tag_id.to_string()],
        )?;
    }
    Ok(())
}
