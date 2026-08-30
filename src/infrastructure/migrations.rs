use rusqlite::Connection;

use super::RepositoryError;

const INITIAL_MIGRATION: &str = include_str!("../../migrations/0001_initial.sql");
const MERGE_INBOX_INTO_TODO: &str = include_str!("../../migrations/0002_merge_inbox_into_todo.sql");
pub const CURRENT_SCHEMA_VERSION: i64 = 2;

pub fn migrate(connection: &mut Connection) -> Result<(), RepositoryError> {
    let current: i64 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if current > CURRENT_SCHEMA_VERSION {
        return Err(RepositoryError::NewerSchema {
            found: current,
            supported: CURRENT_SCHEMA_VERSION,
        });
    }

    if current == 0 {
        let transaction = connection.transaction()?;
        transaction.execute_batch(INITIAL_MIGRATION)?;
        transaction.commit()?;
    } else if current < 2 {
        let transaction = connection.transaction()?;
        transaction.execute_batch(MERGE_INBOX_INTO_TODO)?;
        transaction.commit()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_one_inbox_tasks_are_migrated_to_todo() {
        let mut connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE tasks (status TEXT NOT NULL);
                 INSERT INTO tasks (status) VALUES ('inbox'), ('todo');
                 PRAGMA user_version = 1;",
            )
            .unwrap();

        migrate(&mut connection).unwrap();

        let inbox_count: i64 = connection
            .query_row(
                "SELECT count(*) FROM tasks WHERE status = 'inbox'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let todo_count: i64 = connection
            .query_row(
                "SELECT count(*) FROM tasks WHERE status = 'todo'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let version: i64 = connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(inbox_count, 0);
        assert_eq!(todo_count, 2);
        assert_eq!(version, CURRENT_SCHEMA_VERSION);
    }
}
