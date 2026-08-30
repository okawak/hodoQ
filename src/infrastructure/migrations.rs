use rusqlite::Connection;

use super::RepositoryError;

const INITIAL_MIGRATION: &str = include_str!("../../migrations/0001_initial.sql");
pub const CURRENT_SCHEMA_VERSION: i64 = 1;

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
    }
    Ok(())
}
