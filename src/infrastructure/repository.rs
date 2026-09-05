use std::{
    cmp::Ordering,
    collections::HashMap,
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
    str::FromStr,
    time::Duration,
};

use rusqlite::{Connection, OpenFlags, OptionalExtension, Transaction, params, types::Type};
use serde::Serialize;
use thiserror::Error;
use time::{Date, OffsetDateTime, format_description::well_known::Iso8601};

use crate::domain::{
    Due, DueScope, GroupBy, Priority, Project, ProjectId, SavedBaseView, SavedView, SavedViewId,
    SortDirection, SortField, SortSpec, Tag, TagId, Task, TaskFilter, TaskId, TaskStatus, ViewKind,
};

use super::migrations;

pub struct SqliteRepository {
    connection: Connection,
    path: Option<PathBuf>,
}

impl SqliteRepository {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, RepositoryError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let connection = Connection::open(path)?;
        Self::initialize(connection, Some(path.to_path_buf()))
    }

    pub fn open_in_memory() -> Result<Self, RepositoryError> {
        Self::initialize(Connection::open_in_memory()?, None)
    }

    pub fn open_read_only(path: impl AsRef<Path>) -> Result<Self, RepositoryError> {
        let path = path.as_ref();
        let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        connection.busy_timeout(Duration::from_secs(3))?;
        Ok(Self {
            connection,
            path: Some(path.to_path_buf()),
        })
    }

    fn initialize(
        mut connection: Connection,
        path: Option<PathBuf>,
    ) -> Result<Self, RepositoryError> {
        connection.pragma_update(None, "foreign_keys", "ON")?;
        connection.pragma_update(None, "journal_mode", "DELETE")?;
        connection.busy_timeout(Duration::from_secs(3))?;
        if let Some(database_path) = path.as_deref() {
            let version: i64 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
            let object_count: i64 = connection.query_row(
                "SELECT count(*) FROM sqlite_master WHERE type IN ('table', 'index')",
                [],
                |row| row.get(0),
            )?;
            if version < migrations::CURRENT_SCHEMA_VERSION && object_count > 0 {
                let backup_directory = database_path
                    .parent()
                    .unwrap_or_else(|| Path::new("."))
                    .join("backups");
                fs::create_dir_all(&backup_directory)?;
                let destination = backup_directory.join(format!(
                    "hodoq-before-migration-{}.sqlite3",
                    OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000
                ));
                let mut destination_connection = Connection::open(destination)?;
                let backup =
                    rusqlite::backup::Backup::new(&connection, &mut destination_connection)?;
                backup.run_to_completion(16, Duration::from_millis(20), None)?;
            }
        }
        migrations::migrate(&mut connection)?;
        Ok(Self { connection, path })
    }

    pub fn database_path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

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
        tasks.retain(|task| task_matches(task, filter));
        sort_tasks(&mut tasks, sort);
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

    pub fn save_project(&mut self, project: &Project) -> Result<(), RepositoryError> {
        save_project_on_connection(&self.connection, project)
    }

    pub fn save_projects(&mut self, projects: &[Project]) -> Result<(), RepositoryError> {
        let transaction = self.connection.transaction()?;
        for project in projects {
            save_project_on_connection(&transaction, project)?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn apply_history_state(
        &mut self,
        tasks: Option<&[Task]>,
        projects_to_save: &[Project],
        projects_to_delete: &[ProjectId],
        tags_to_save: &[Tag],
        tags_to_delete: &[TagId],
    ) -> Result<(), RepositoryError> {
        let transaction = self.connection.transaction()?;
        for project in projects_to_save {
            save_project_on_connection(&transaction, project)?;
        }
        for tag in tags_to_save {
            save_tag_on_connection(&transaction, tag)?;
        }
        if let Some(tasks) = tasks {
            transaction.execute("DELETE FROM tasks", [])?;
            for task in tasks {
                save_task_in_transaction(&transaction, task)?;
            }
        }
        for id in projects_to_delete {
            transaction.execute("DELETE FROM projects WHERE id = ?1", [id.to_string()])?;
        }
        for id in tags_to_delete {
            transaction.execute("DELETE FROM tags WHERE id = ?1", [id.to_string()])?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn list_projects(&self) -> Result<Vec<Project>, RepositoryError> {
        let mut statement = self.connection.prepare(
            "SELECT id, name, description, color, sort_order, created_at, updated_at, archived_at
             FROM projects ORDER BY sort_order, name COLLATE NOCASE",
        )?;
        statement
            .query_map([], |row| {
                Ok(Project {
                    id: parse_id(row.get::<_, String>(0)?, 0)?,
                    name: row.get(1)?,
                    description: row.get(2)?,
                    color: row.get(3)?,
                    sort_order: row.get(4)?,
                    created_at: parse_timestamp(row.get(5)?, 5)?,
                    updated_at: parse_timestamp(row.get(6)?, 6)?,
                    archived_at: parse_optional_timestamp(row.get(7)?, 7)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    pub fn delete_project(&mut self, id: ProjectId) -> Result<bool, RepositoryError> {
        Ok(self
            .connection
            .execute("DELETE FROM projects WHERE id = ?1", [id.to_string()])?
            > 0)
    }

    pub fn save_tag(&mut self, tag: &Tag) -> Result<(), RepositoryError> {
        save_tag_on_connection(&self.connection, tag)
    }

    pub fn list_tags(&self) -> Result<Vec<Tag>, RepositoryError> {
        let mut statement = self
            .connection
            .prepare("SELECT id, name, color, created_at FROM tags ORDER BY name COLLATE NOCASE")?;
        statement
            .query_map([], |row| {
                Ok(Tag {
                    id: parse_id(row.get::<_, String>(0)?, 0)?,
                    name: row.get(1)?,
                    color: row.get(2)?,
                    created_at: parse_timestamp(row.get(3)?, 3)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    pub fn delete_tag(&mut self, id: TagId) -> Result<bool, RepositoryError> {
        Ok(self
            .connection
            .execute("DELETE FROM tags WHERE id = ?1", [id.to_string()])?
            > 0)
    }

    pub fn save_view(&mut self, view: &SavedView) -> Result<(), RepositoryError> {
        let filter_json = serde_json::to_string(&view.filter)?;
        let sort_json = serde_json::to_string(&view.sort)?;
        let group_by = view
            .group_by
            .map(|group| serde_json::to_string(&group))
            .transpose()?;
        self.connection.execute(
            "INSERT INTO saved_views
             (id, name, view_kind, filter_json, sort_json, group_by, sort_order, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(id) DO UPDATE SET
               name = excluded.name,
               view_kind = excluded.view_kind,
               filter_json = excluded.filter_json,
               sort_json = excluded.sort_json,
               group_by = excluded.group_by,
               sort_order = excluded.sort_order,
               updated_at = excluded.updated_at",
            params![
                view.id.to_string(),
                view.name,
                view.view_kind.as_str(),
                filter_json,
                sort_json,
                group_by,
                view.sort_order,
                timestamp_millis(view.created_at),
                timestamp_millis(view.updated_at),
            ],
        )?;
        Ok(())
    }

    pub fn list_views(&self) -> Result<Vec<SavedView>, RepositoryError> {
        let mut statement = self.connection.prepare(
            "SELECT id, name, view_kind, filter_json, sort_json, group_by, sort_order,
                    created_at, updated_at
             FROM saved_views ORDER BY sort_order, name COLLATE NOCASE",
        )?;
        let rows = statement.query_map([], |row| {
            let view_kind: String = row.get(2)?;
            let filter_json: String = row.get(3)?;
            let sort_json: String = row.get(4)?;
            let group_json: Option<String> = row.get(5)?;
            Ok(SavedView {
                id: parse_id(row.get::<_, String>(0)?, 0)?,
                name: row.get(1)?,
                view_kind: ViewKind::from_str(&view_kind)
                    .map_err(|error| conversion_error(2, StringError(error)))?,
                filter: serde_json::from_str(&filter_json)
                    .map_err(|error| conversion_error(3, error))?,
                sort: serde_json::from_str(&sort_json)
                    .map_err(|error| conversion_error(4, error))?,
                group_by: group_json
                    .map(|value| serde_json::from_str::<GroupBy>(&value))
                    .transpose()
                    .map_err(|error| conversion_error(5, error))?,
                sort_order: row.get(6)?,
                created_at: parse_timestamp(row.get(7)?, 7)?,
                updated_at: parse_timestamp(row.get(8)?, 8)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn delete_view(&mut self, id: SavedViewId) -> Result<bool, RepositoryError> {
        Ok(self
            .connection
            .execute("DELETE FROM saved_views WHERE id = ?1", [id.to_string()])?
            > 0)
    }

    pub fn create_backup(&self, destination: &Path) -> Result<(), RepositoryError> {
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut destination_connection = Connection::open(destination)?;
        let backup = rusqlite::backup::Backup::new(&self.connection, &mut destination_connection)?;
        backup.run_to_completion(16, Duration::from_millis(20), None)?;
        Ok(())
    }

    pub fn integrity_check(path: &Path) -> Result<bool, RepositoryError> {
        let connection =
            Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let result: String =
            connection.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
        Ok(result == "ok")
    }

    pub fn restore_from_backup(
        &mut self,
        source: &Path,
        safety_backup: &Path,
    ) -> Result<(), RepositoryError> {
        if !Self::integrity_check(source)? {
            return Err(RepositoryError::InvalidBackup(
                "integrity_check failed".to_owned(),
            ));
        }
        let source_connection =
            Connection::open_with_flags(source, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let source_version: i64 =
            source_connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if source_version > migrations::CURRENT_SCHEMA_VERSION {
            return Err(RepositoryError::NewerSchema {
                found: source_version,
                supported: migrations::CURRENT_SCHEMA_VERSION,
            });
        }
        self.create_backup(safety_backup)?;
        {
            let backup = rusqlite::backup::Backup::new(&source_connection, &mut self.connection)?;
            backup.run_to_completion(16, Duration::from_millis(20), None)?;
        }
        self.connection.pragma_update(None, "foreign_keys", "ON")?;
        migrations::migrate(&mut self.connection)?;
        Ok(())
    }

    pub fn export_csv(
        &self,
        destination: &Path,
        filter: &TaskFilter,
        with_bom: bool,
    ) -> Result<(), RepositoryError> {
        let tasks = self.list_tasks(filter, &[])?;
        Self::export_tasks_csv(destination, &tasks, with_bom)
    }

    pub fn export_tasks_csv(
        destination: &Path,
        tasks: &[Task],
        with_bom: bool,
    ) -> Result<(), RepositoryError> {
        let mut file = File::create(destination)?;
        if with_bom {
            file.write_all(&[0xEF, 0xBB, 0xBF])?;
        }
        let mut writer = csv::Writer::from_writer(file);
        writer.write_record([
            "id",
            "title",
            "memo",
            "status",
            "priority",
            "progress",
            "due",
            "project_id",
            "tag_ids",
            "created_at",
            "updated_at",
        ])?;
        for task in tasks {
            writer.write_record([
                task.id.to_string(),
                task.title.clone(),
                task.memo.clone(),
                task.status.as_str().to_owned(),
                task.priority.as_str().to_owned(),
                task.progress.to_string(),
                due_display_value(&task.due)?,
                task.project_id.map(|id| id.to_string()).unwrap_or_default(),
                task.tag_ids
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(";"),
                task.created_at.format(&Iso8601::DEFAULT)?,
                task.updated_at.format(&Iso8601::DEFAULT)?,
            ])?;
        }
        writer.flush()?;
        Ok(())
    }

    pub fn export_json(&self, destination: &Path) -> Result<(), RepositoryError> {
        let data = ExportData {
            format_version: 1,
            exported_at: OffsetDateTime::now_utc(),
            tasks: self.list_all_tasks()?,
            projects: self.list_projects()?,
            tags: self.list_tags()?,
            saved_views: self.list_views()?,
        };
        fs::write(destination, serde_json::to_vec_pretty(&data)?)?;
        Ok(())
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

fn save_project_on_connection(
    connection: &Connection,
    project: &Project,
) -> Result<(), RepositoryError> {
    connection.execute(
        "INSERT INTO projects
         (id, name, description, color, sort_order, created_at, updated_at, archived_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(id) DO UPDATE SET
           name = excluded.name,
           description = excluded.description,
           color = excluded.color,
           sort_order = excluded.sort_order,
           updated_at = excluded.updated_at,
           archived_at = excluded.archived_at",
        params![
            project.id.to_string(),
            project.name,
            project.description,
            project.color,
            project.sort_order,
            timestamp_millis(project.created_at),
            timestamp_millis(project.updated_at),
            project.archived_at.map(timestamp_millis),
        ],
    )?;
    Ok(())
}

fn save_tag_on_connection(connection: &Connection, tag: &Tag) -> Result<(), RepositoryError> {
    connection.execute(
        "INSERT INTO tags (id, name, color, created_at)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(id) DO UPDATE SET name = excluded.name, color = excluded.color",
        params![
            tag.id.to_string(),
            tag.name,
            tag.color,
            timestamp_millis(tag.created_at)
        ],
    )?;
    Ok(())
}

fn save_task_in_transaction(
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

fn row_to_task(row: &rusqlite::Row<'_>) -> rusqlite::Result<Task> {
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

fn task_matches(task: &Task, filter: &TaskFilter) -> bool {
    let visibility_matches = match filter.base_view {
        Some(SavedBaseView::Trash) => task.deleted_at.is_some(),
        _ if filter.only_deleted => task.deleted_at.is_some(),
        _ => task.deleted_at.is_none(),
    };
    if !visibility_matches {
        return false;
    }
    if !filter.include_archived
        && !filter.only_deleted
        && filter.base_view != Some(SavedBaseView::Archived)
        && filter.base_view != Some(SavedBaseView::Trash)
        && task.status == TaskStatus::Archived
    {
        return false;
    }
    let query = filter.query.trim().to_lowercase();
    if !query.is_empty()
        && !task.title.to_lowercase().contains(&query)
        && !task.memo.to_lowercase().contains(&query)
    {
        return false;
    }
    if !status_filter_matches(&filter.statuses, task.status) {
        return false;
    }
    if !filter.priorities.is_empty() && !filter.priorities.contains(&task.priority) {
        return false;
    }
    if !filter.project_ids.is_empty() || filter.unassigned_project {
        let matched = task.project_id.map_or(filter.unassigned_project, |id| {
            filter.project_ids.contains(&id)
        });
        if !matched {
            return false;
        }
    }
    if !filter.tag_ids.is_empty() {
        let matched = if filter.match_all_tags {
            filter.tag_ids.iter().all(|id| task.tag_ids.contains(id))
        } else {
            filter.tag_ids.iter().any(|id| task.tag_ids.contains(id))
        };
        if !matched {
            return false;
        }
    }
    let now = OffsetDateTime::now_utc();
    let offset = time::UtcOffset::current_local_offset().unwrap_or(time::UtcOffset::UTC);
    let today = now.to_offset(offset).date();
    let due_date = match &task.due {
        Due::None => None,
        Due::Date(date) => Some(*date),
        Due::DateTime(date_time) => Some(date_time.to_offset(offset).date()),
    };
    let smart_view_matches = match filter.base_view {
        None | Some(SavedBaseView::Trash) => true,
        Some(SavedBaseView::Inbox) => task.status == TaskStatus::Todo,
        Some(SavedBaseView::Today) => due_date == Some(today),
        Some(SavedBaseView::Upcoming) => {
            due_date.is_some_and(|date| date >= today && date <= today + time::Duration::days(7))
        }
        Some(SavedBaseView::Overdue) => {
            task.status != TaskStatus::Done && task.due.is_overdue(now, today)
        }
        Some(SavedBaseView::Undated) => due_date.is_none(),
        Some(SavedBaseView::Doing) => task.status == TaskStatus::Doing,
        Some(SavedBaseView::Blocked) => task.status == TaskStatus::Blocked,
        Some(SavedBaseView::Done) => task.status == TaskStatus::Done,
        Some(SavedBaseView::Archived) => task.status == TaskStatus::Archived,
        Some(SavedBaseView::Project(id)) => task.project_id == Some(id),
        Some(SavedBaseView::Tag(id)) => task.tag_ids.contains(&id),
    };
    if !smart_view_matches {
        return false;
    }
    let due_scope_matches = match filter.due_scope {
        DueScope::Any => true,
        DueScope::Undated => due_date.is_none(),
        DueScope::Today => due_date == Some(today),
        DueScope::Upcoming => {
            due_date.is_some_and(|date| date >= today && date <= today + time::Duration::days(7))
        }
        DueScope::Overdue => task.status != TaskStatus::Done && task.due.is_overdue(now, today),
    };
    if !due_scope_matches {
        return false;
    }
    if let Some(from) = filter.due_from {
        let from_date = from.to_offset(offset).date();
        if due_date.is_none_or(|date| date < from_date) {
            return false;
        }
    }
    if let Some(to) = filter.due_to {
        let to_date = to.to_offset(offset).date();
        if due_date.is_none_or(|date| date > to_date) {
            return false;
        }
    }
    true
}

fn status_filter_matches(statuses: &[TaskStatus], status: TaskStatus) -> bool {
    statuses.is_empty()
        || statuses.contains(&status)
        || (status == TaskStatus::Todo && statuses.contains(&TaskStatus::Inbox))
}

fn sort_tasks(tasks: &mut [Task], sort: &[SortSpec]) {
    let specs = if sort.is_empty() {
        vec![SortSpec::default()]
    } else {
        sort.to_vec()
    };
    tasks.sort_by(|left, right| {
        for spec in &specs {
            let ordering = match spec.field {
                SortField::Manual => left.sort_order.cmp(&right.sort_order),
                SortField::Priority => left.priority.cmp(&right.priority),
                SortField::Due => compare_due(&left.due, &right.due),
                SortField::UpdatedAt => left.updated_at.cmp(&right.updated_at),
                SortField::CreatedAt => left.created_at.cmp(&right.created_at),
                SortField::Title => left.title.to_lowercase().cmp(&right.title.to_lowercase()),
            };
            let ordering = match spec.direction {
                SortDirection::Ascending => ordering,
                SortDirection::Descending => ordering.reverse(),
            };
            if ordering != Ordering::Equal {
                return ordering;
            }
        }
        left.id.to_string().cmp(&right.id.to_string())
    });
}

fn compare_due(left: &Due, right: &Due) -> Ordering {
    match (left, right) {
        (Due::None, Due::None) => Ordering::Equal,
        (Due::None, _) => Ordering::Greater,
        (_, Due::None) => Ordering::Less,
        (Due::Date(left), Due::Date(right)) => left.cmp(right),
        (Due::DateTime(left), Due::DateTime(right)) => left.cmp(right),
        (Due::Date(left), Due::DateTime(right)) => left.cmp(
            &right
                .to_offset(time::UtcOffset::current_local_offset().unwrap_or(time::UtcOffset::UTC))
                .date(),
        ),
        (Due::DateTime(left), Due::Date(right)) => left
            .to_offset(time::UtcOffset::current_local_offset().unwrap_or(time::UtcOffset::UTC))
            .date()
            .cmp(right),
    }
}

fn due_columns(due: &Due) -> Result<(&'static str, Option<String>, Option<i64>), RepositoryError> {
    match due {
        Due::None => Ok(("none", None, None)),
        Due::Date(date) => Ok(("date", Some(date.format(&Iso8601::DATE)?), None)),
        Due::DateTime(date_time) => Ok(("datetime", None, Some(timestamp_millis(*date_time)))),
    }
}

fn due_display_value(due: &Due) -> Result<String, RepositoryError> {
    match due {
        Due::None => Ok(String::new()),
        Due::Date(date) => Ok(date.format(&Iso8601::DATE)?),
        Due::DateTime(date_time) => Ok(date_time.format(&Iso8601::DEFAULT)?),
    }
}

fn timestamp_millis(value: OffsetDateTime) -> i64 {
    (value.unix_timestamp_nanos() / 1_000_000) as i64
}

fn parse_timestamp(value: i64, index: usize) -> rusqlite::Result<OffsetDateTime> {
    OffsetDateTime::from_unix_timestamp_nanos(i128::from(value) * 1_000_000)
        .map_err(|error| conversion_error(index, error))
}

fn parse_optional_timestamp(
    value: Option<i64>,
    index: usize,
) -> rusqlite::Result<Option<OffsetDateTime>> {
    value.map(|value| parse_timestamp(value, index)).transpose()
}

fn parse_id<T>(value: String, index: usize) -> rusqlite::Result<T>
where
    T: FromStr,
    T::Err: std::error::Error + Send + Sync + 'static,
{
    T::from_str(&value).map_err(|error| conversion_error(index, error))
}

fn conversion_error(
    index: usize,
    error: impl std::error::Error + Send + Sync + 'static,
) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(index, Type::Text, Box::new(error))
}

#[derive(Debug, Error)]
#[error("{0}")]
struct StringError(String);

#[derive(Serialize)]
struct ExportData {
    format_version: u32,
    exported_at: OffsetDateTime,
    tasks: Vec<Task>,
    projects: Vec<Project>,
    tags: Vec<Tag>,
    saved_views: Vec<SavedView>,
}

#[derive(Debug, Error)]
pub enum RepositoryError {
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("file error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("CSV error: {0}")]
    Csv(#[from] csv::Error),
    #[error("date formatting error: {0}")]
    DateFormat(#[from] time::error::Format),
    #[error("application data directory is unavailable")]
    DataDirectoryUnavailable,
    #[error(
        "HodoQ repository root could not be found; run the executable inside the cloned repository or pass --data-dir"
    )]
    RepositoryRootUnavailable,
    #[error("HodoQ is already running with this data directory")]
    AlreadyRunning,
    #[error("database worker stopped")]
    WorkerStopped,
    #[error("database worker could not start: {0}")]
    WorkerInitialization(String),
    #[error("database operation failed: {0}")]
    WorkerOperation(String),
    #[error("database is open in read-only recovery mode")]
    ReadOnly,
    #[error("database schema {found} is newer than supported schema {supported}")]
    NewerSchema { found: i64, supported: i64 },
    #[error("invalid backup: {0}")]
    InvalidBackup(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_round_trip_preserves_due_variants() {
        let mut repository = SqliteRepository::open_in_memory().unwrap();
        let now = OffsetDateTime::UNIX_EPOCH;
        let mut task = Task::new("日付タスク", now).unwrap();
        task.due = Due::Date(Date::from_calendar_date(2026, time::Month::August, 27).unwrap());
        repository.save_task(&task).unwrap();
        assert_eq!(repository.task(task.id).unwrap().unwrap(), task);

        task.due = Due::DateTime(now + time::Duration::hours(3));
        repository.save_task(&task).unwrap();
        assert_eq!(repository.task(task.id).unwrap().unwrap(), task);
    }

    #[test]
    fn trash_is_purged_only_after_retention_period() {
        let mut repository = SqliteRepository::open_in_memory().unwrap();
        let now = OffsetDateTime::UNIX_EPOCH + time::Duration::days(100);
        let mut recent = Task::new("recent", now).unwrap();
        recent.deleted_at = Some(now - time::Duration::days(29));
        repository.save_task(&recent).unwrap();
        let mut expired = Task::new("expired", now).unwrap();
        expired.deleted_at = Some(now - time::Duration::days(30));
        repository.save_task(&expired).unwrap();

        assert_eq!(repository.purge_expired_trash(now, 30).unwrap(), 1);
        assert!(repository.task(recent.id).unwrap().is_some());
        assert!(repository.task(expired.id).unwrap().is_none());
    }

    #[test]
    fn deleting_project_keeps_task() {
        let mut repository = SqliteRepository::open_in_memory().unwrap();
        let now = OffsetDateTime::UNIX_EPOCH;
        let project = Project::new("project", now);
        repository.save_project(&project).unwrap();
        let mut task = Task::new("task", now).unwrap();
        task.project_id = Some(project.id);
        repository.save_task(&task).unwrap();

        repository.delete_project(project.id).unwrap();
        let task = repository.task(task.id).unwrap().unwrap();
        assert_eq!(task.project_id, None);
    }

    #[test]
    fn foreign_keys_reject_unknown_project_and_tag_without_partial_task() {
        let mut repository = SqliteRepository::open_in_memory().unwrap();
        let now = OffsetDateTime::UNIX_EPOCH;
        let unknown_project = Project::new("not saved", now);
        let unknown_tag = Tag::new("not saved", now);
        let mut task = Task::new("task", now).unwrap();
        task.project_id = Some(unknown_project.id);
        task.tag_ids.push(unknown_tag.id);

        assert!(repository.save_task(&task).is_err());
        assert!(repository.task(task.id).unwrap().is_none());
    }

    #[test]
    fn batch_save_is_atomic() {
        let mut repository = SqliteRepository::open_in_memory().unwrap();
        let now = OffsetDateTime::UNIX_EPOCH;
        let valid = Task::new("valid", now).unwrap();
        let mut invalid = Task::new("invalid", now).unwrap();
        invalid.title.clear();

        assert!(repository.save_tasks(&[valid.clone(), invalid]).is_err());
        assert!(repository.task(valid.id).unwrap().is_none());
    }

    #[test]
    fn project_batch_save_is_atomic() {
        let mut repository = SqliteRepository::open_in_memory().unwrap();
        let now = OffsetDateTime::UNIX_EPOCH;
        let valid = Project::new("valid", now);
        let invalid = Project::new("", now);

        assert!(repository.save_projects(&[valid.clone(), invalid]).is_err());
        assert!(repository.list_projects().unwrap().is_empty());
    }

    #[test]
    fn history_state_is_atomic_across_related_entities() {
        let mut repository = SqliteRepository::open_in_memory().unwrap();
        let now = OffsetDateTime::UNIX_EPOCH;
        let project = Project::new("project", now);
        let mut invalid_task = Task::new("task", now).unwrap();
        invalid_task.title.clear();
        invalid_task.project_id = Some(project.id);

        assert!(
            repository
                .apply_history_state(
                    Some(&[invalid_task]),
                    std::slice::from_ref(&project),
                    &[],
                    &[],
                    &[],
                )
                .is_err()
        );
        assert!(repository.list_projects().unwrap().is_empty());
        assert!(repository.list_all_tasks().unwrap().is_empty());
    }

    #[test]
    fn history_state_restores_project_tag_and_task_relationships_together() {
        let mut repository = SqliteRepository::open_in_memory().unwrap();
        let now = OffsetDateTime::UNIX_EPOCH;
        let project = Project::new("project", now);
        let tag = Tag::new("tag", now);
        let mut task = Task::new("task", now).unwrap();
        task.project_id = Some(project.id);
        task.tag_ids.push(tag.id);

        repository
            .apply_history_state(
                Some(&[task.clone()]),
                std::slice::from_ref(&project),
                &[],
                std::slice::from_ref(&tag),
                &[],
            )
            .unwrap();

        assert_eq!(repository.list_projects().unwrap(), vec![project]);
        assert_eq!(repository.list_tags().unwrap(), vec![tag]);
        assert_eq!(repository.task(task.id).unwrap(), Some(task));
    }

    #[test]
    fn list_all_tasks_includes_trash() {
        let mut repository = SqliteRepository::open_in_memory().unwrap();
        let now = OffsetDateTime::UNIX_EPOCH;
        let active = Task::new("active", now).unwrap();
        let mut deleted = Task::new("deleted", now).unwrap();
        deleted.move_to_trash(now);
        repository.save_tasks(&[active, deleted]).unwrap();

        let tasks = repository.list_all_tasks().unwrap();
        assert_eq!(tasks.len(), 2);
        assert_eq!(
            tasks
                .iter()
                .filter(|task| task.deleted_at.is_some())
                .count(),
            1
        );
    }

    #[test]
    fn verified_backup_can_restore_all_data() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("tasks.sqlite3");
        let backup = directory.path().join("backup.sqlite3");
        let safety = directory.path().join("before-restore.sqlite3");
        let mut repository = SqliteRepository::open(&database).unwrap();
        let now = OffsetDateTime::UNIX_EPOCH;
        let project = Project::new("project", now);
        let tag = Tag::new("tag", now);
        let view = SavedView {
            id: SavedViewId::new(),
            name: "view".to_owned(),
            view_kind: ViewKind::List,
            filter: TaskFilter::default(),
            sort: vec![SortSpec::default()],
            group_by: None,
            sort_order: 0,
            created_at: now,
            updated_at: now,
        };
        let mut task = Task::new("before", now).unwrap();
        task.project_id = Some(project.id);
        task.tag_ids.push(tag.id);
        repository.save_project(&project).unwrap();
        repository.save_tag(&tag).unwrap();
        repository.save_view(&view).unwrap();
        repository.save_task(&task).unwrap();
        repository.create_backup(&backup).unwrap();

        task.set_title("after").unwrap();
        repository.save_task(&task).unwrap();
        repository.delete_view(view.id).unwrap();
        repository.delete_tag(tag.id).unwrap();
        repository.delete_project(project.id).unwrap();
        repository.restore_from_backup(&backup, &safety).unwrap();

        task.set_title("before").unwrap();
        assert_eq!(repository.task(task.id).unwrap(), Some(task));
        assert_eq!(repository.list_projects().unwrap(), vec![project]);
        assert_eq!(repository.list_tags().unwrap(), vec![tag]);
        assert_eq!(repository.list_views().unwrap(), vec![view]);
        assert!(SqliteRepository::integrity_check(&safety).unwrap());
    }

    #[test]
    fn newer_backup_is_rejected_without_changing_current_data() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("tasks.sqlite3");
        let source = directory.path().join("newer.sqlite3");
        let safety = directory.path().join("safety.sqlite3");
        let mut repository = SqliteRepository::open(&database).unwrap();
        let task = Task::new("current", OffsetDateTime::UNIX_EPOCH).unwrap();
        repository.save_task(&task).unwrap();
        let source_connection = Connection::open(&source).unwrap();
        source_connection
            .pragma_update(None, "user_version", 999)
            .unwrap();
        drop(source_connection);

        assert!(matches!(
            repository.restore_from_backup(&source, &safety),
            Err(RepositoryError::NewerSchema { .. })
        ));
        assert_eq!(repository.task(task.id).unwrap(), Some(task));
        assert!(!safety.exists());
    }

    #[test]
    fn saved_view_round_trip_preserves_conditions() {
        let mut repository = SqliteRepository::open_in_memory().unwrap();
        let now = OffsetDateTime::UNIX_EPOCH;
        let project = Project::new("project", now);
        let view = SavedView {
            id: SavedViewId::new(),
            name: "重要".to_owned(),
            view_kind: ViewKind::Board,
            filter: TaskFilter {
                base_view: Some(SavedBaseView::Project(project.id)),
                priorities: vec![Priority::High],
                ..TaskFilter::default()
            },
            sort: vec![SortSpec {
                field: SortField::Due,
                direction: SortDirection::Ascending,
            }],
            group_by: Some(GroupBy::Status),
            sort_order: 0,
            created_at: now,
            updated_at: now,
        };
        repository.save_view(&view).unwrap();
        assert_eq!(repository.list_views().unwrap(), vec![view]);
    }

    #[test]
    fn due_scope_filter_selects_undated_tasks() {
        let mut repository = SqliteRepository::open_in_memory().unwrap();
        let now = OffsetDateTime::now_utc();
        let undated = Task::new("undated", now).unwrap();
        let mut dated = Task::new("dated", now).unwrap();
        dated.due = Due::Date(now.date());
        repository.save_tasks(&[undated.clone(), dated]).unwrap();

        let tasks = repository
            .list_tasks(
                &TaskFilter {
                    due_scope: DueScope::Undated,
                    ..TaskFilter::default()
                },
                &[],
            )
            .unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].id, undated.id);
    }

    #[test]
    fn overdue_filter_excludes_completed_tasks() {
        let mut repository = SqliteRepository::open_in_memory().unwrap();
        let now = OffsetDateTime::now_utc();
        let yesterday = now.date() - time::Duration::days(1);
        let mut open = Task::new("open", now).unwrap();
        open.due = Due::Date(yesterday);
        let mut done = Task::new("done", now).unwrap();
        done.due = Due::Date(yesterday);
        done.set_status(TaskStatus::Done, now);
        repository.save_tasks(&[open.clone(), done]).unwrap();

        let tasks = repository
            .list_tasks(
                &TaskFilter {
                    due_scope: DueScope::Overdue,
                    ..TaskFilter::default()
                },
                &[],
            )
            .unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].id, open.id);
    }

    #[test]
    fn existing_schema_is_backed_up_before_migration() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("tasks.sqlite3");
        let connection = Connection::open(&database).unwrap();
        connection
            .execute("CREATE TABLE legacy (value TEXT NOT NULL)", [])
            .unwrap();
        drop(connection);

        SqliteRepository::open(&database).unwrap();

        let backups = fs::read_dir(directory.path().join("backups"))
            .unwrap()
            .filter_map(Result::ok)
            .collect::<Vec<_>>();
        assert_eq!(backups.len(), 1);
        assert!(SqliteRepository::integrity_check(&backups[0].path()).unwrap());
    }

    #[test]
    fn ten_thousand_tasks_can_be_saved_and_loaded() {
        check_ten_thousand_task_round_trip();
    }

    #[test]
    #[ignore = "run performance_ tests in release mode with --test-threads=1"]
    #[allow(clippy::assertions_on_constants)]
    fn performance_ten_thousand_task_round_trip() {
        // An explicit --ignored debug run should fail, not report misleading timings.
        assert!(
            !cfg!(debug_assertions),
            "performance tests require --release"
        );
        let (round_trip, load, search) = check_ten_thousand_task_round_trip();
        eprintln!("10,000 tasks: round trip={round_trip:?}, load={load:?}, search={search:?}");
        assert!(
            round_trip < Duration::from_secs(5),
            "10,000 task round trip took {round_trip:?}"
        );
        assert!(
            load < Duration::from_secs(1),
            "10,000 task load took {load:?}"
        );
        assert!(
            search < Duration::from_millis(100),
            "10,000 task search took {search:?}"
        );
    }

    // Keep the same dataset and correctness checks in functional and performance runs.
    fn check_ten_thousand_task_round_trip() -> (Duration, Duration, Duration) {
        let mut repository = SqliteRepository::open_in_memory().unwrap();
        let now = OffsetDateTime::UNIX_EPOCH;
        let tasks = (0..10_000)
            .map(|index| Task::new(format!("task {index}"), now).unwrap())
            .collect::<Vec<_>>();
        let started = std::time::Instant::now();
        repository.save_tasks(&tasks).unwrap();
        let load_started = std::time::Instant::now();
        let loaded = repository.list_all_tasks().unwrap();
        let load_elapsed = load_started.elapsed();
        let round_trip_elapsed = started.elapsed();

        assert_eq!(loaded.len(), 10_000);
        let search_started = std::time::Instant::now();
        let filter = TaskFilter {
            query: "task 9999".to_owned(),
            ..TaskFilter::default()
        };
        let matches = repository
            .list_tasks(&filter, &[SortSpec::default()])
            .unwrap();
        let search_elapsed = search_started.elapsed();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].id, tasks[9999].id);
        (round_trip_elapsed, load_elapsed, search_elapsed)
    }

    #[test]
    fn project_filter_can_include_selected_and_unassigned_tasks() {
        let now = OffsetDateTime::UNIX_EPOCH;
        let project = Project::new("project", now);
        let mut assigned = Task::new("assigned", now).unwrap();
        assigned.project_id = Some(project.id);
        let unassigned = Task::new("unassigned", now).unwrap();
        let filter = TaskFilter {
            project_ids: vec![project.id],
            unassigned_project: true,
            ..TaskFilter::default()
        };
        assert!(task_matches(&assigned, &filter));
        assert!(task_matches(&unassigned, &filter));
    }

    #[test]
    fn saved_base_views_preserve_project_archive_and_trash_scope() {
        let now = OffsetDateTime::UNIX_EPOCH;
        let project = Project::new("project", now);
        let mut assigned = Task::new("assigned", now).unwrap();
        assigned.project_id = Some(project.id);
        let unassigned = Task::new("unassigned", now).unwrap();
        let project_filter = TaskFilter {
            base_view: Some(SavedBaseView::Project(project.id)),
            ..TaskFilter::default()
        };
        assert!(task_matches(&assigned, &project_filter));
        assert!(!task_matches(&unassigned, &project_filter));

        let mut archived = Task::new("archived", now).unwrap();
        archived.set_status(TaskStatus::Archived, now);
        let archive_filter = TaskFilter {
            base_view: Some(SavedBaseView::Archived),
            ..TaskFilter::default()
        };
        assert!(task_matches(&archived, &archive_filter));

        archived.move_to_trash(now);
        let trash_filter = TaskFilter {
            base_view: Some(SavedBaseView::Trash),
            ..TaskFilter::default()
        };
        assert!(task_matches(&archived, &trash_filter));
    }

    #[test]
    fn csv_export_has_optional_bom_and_consistent_columns() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("tasks.csv");
        let mut task = Task::new("comma, title", OffsetDateTime::UNIX_EPOCH).unwrap();
        task.memo = "line 1\nline 2".to_owned();
        SqliteRepository::export_tasks_csv(&path, &[task], true).unwrap();
        let bytes = fs::read(&path).unwrap();
        assert!(bytes.starts_with(&[0xEF, 0xBB, 0xBF]));
        let mut reader = csv::Reader::from_reader(bytes.as_slice());
        assert_eq!(reader.headers().unwrap().len(), 11);
        assert_eq!(reader.records().next().unwrap().unwrap().len(), 11);

        SqliteRepository::export_tasks_csv(&path, &[], false).unwrap();
        assert!(!fs::read(path).unwrap().starts_with(&[0xEF, 0xBB, 0xBF]));
    }
}
