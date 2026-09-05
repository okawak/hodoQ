//! Projects, tags and saved views stored alongside tasks.
use super::{
    RepositoryError, SqliteRepository,
    mapping::{
        StringError, conversion_error, parse_id, parse_optional_timestamp, parse_timestamp,
        timestamp_millis,
    },
};
use crate::domain::{GroupBy, Project, ProjectId, SavedView, SavedViewId, Tag, TagId, ViewKind};
use rusqlite::{Connection, params};
use std::str::FromStr;

impl SqliteRepository {
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
}

pub(super) fn save_project_on_connection(
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

pub(super) fn save_tag_on_connection(
    connection: &Connection,
    tag: &Tag,
) -> Result<(), RepositoryError> {
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
