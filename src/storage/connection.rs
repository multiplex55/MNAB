use std::{path::Path, time::Duration};

use rusqlite::{Connection, OpenFlags};

use super::migration::{MigrationError, migrate};

pub const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// Opens the one writable connection owned by a storage worker.
pub fn open_primary(path: &Path) -> Result<Connection, MigrationError> {
    let mut connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
    )?;
    connection.busy_timeout(BUSY_TIMEOUT)?;
    connection.execute_batch("PRAGMA foreign_keys = ON;")?;
    if connection.query_row("PRAGMA foreign_keys", [], |row| row.get::<_, i64>(0))? != 1 {
        return Err(MigrationError::ForeignKeysUnavailable);
    }

    // WAL is only selected when this SQLite build reports support. Failure is returned: a
    // portable database must never be silently moved or downgraded to another journal mode.
    let mode: String = connection.query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))?;
    if !mode.eq_ignore_ascii_case("wal") {
        return Err(MigrationError::WalUnavailable);
    }
    migrate(&mut connection, path)?;
    Ok(connection)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blank_database_is_migrated_with_connection_invariants() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("budget.sqlite3");
        let connection = open_primary(&path).unwrap();
        assert_eq!(
            connection
                .pragma_query_value(None, "foreign_keys", |r| r.get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            super::super::migration::current_version(&connection).unwrap(),
            super::super::migration::LATEST_SCHEMA_VERSION
        );
        assert!(path.with_file_name("budget.sqlite3-wal").exists());
        assert!(path.with_file_name("budget.sqlite3-shm").exists());
    }

    #[test]
    fn foreign_keys_and_present_fitids_are_unique() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("budget.sqlite3");
        let connection = open_primary(&path).unwrap();
        assert!(connection.execute("INSERT INTO accounts(id,budget_id,name,account_type,sort_order,created_at,modified_at) VALUES(?1,?2,?3,?4,?5,?6,?7)", ("a", "missing", "Checking", "checking", 0, "now", "now")).is_err());
        connection
            .execute(
                "INSERT INTO budgets(id,name,created_at,modified_at) VALUES(?1,?2,?3,?4)",
                ("b", "Budget", "now", "now"),
            )
            .unwrap();
        connection.execute("INSERT INTO accounts(id,budget_id,name,account_type,sort_order,created_at,modified_at) VALUES(?1,?2,?3,?4,?5,?6,?7)", ("a", "b", "Checking", "checking", 0, "now", "now")).unwrap();
        let insert = |id: &str| {
            connection.execute("INSERT INTO transactions(id,budget_id,account_id,transaction_date,amount,cleared_state,approval_state,imported_fitid,created_at,modified_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)", (id,"b","a","2026-01-01",1,"uncleared","approved","fit-1","now","now"))
        };
        insert("t1").unwrap();
        assert!(insert("t2").is_err());
    }
}
