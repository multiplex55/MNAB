use rusqlite::{Connection, OptionalExtension};
use std::{
    fs,
    path::{Path, PathBuf},
};
use thiserror::Error;

pub const LATEST_SCHEMA_VERSION: i64 = 2;
const INITIAL_SQL: &str = include_str!("migrations/0001_initial.sql");
const RECONCILIATION_SQL: &str = include_str!("migrations/0002_reconciliation_history.sql");
const INITIAL_CHECKSUM: &str = "0001-initial-v1";

#[derive(Debug, Error)]
pub enum MigrationError {
    #[error("SQLite operation failed: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("database schema {found} is newer than supported schema {supported}")]
    FutureSchema { found: i64, supported: i64 },
    #[error("foreign-key enforcement could not be enabled")]
    ForeignKeysUnavailable,
    #[error("WAL journaling is unavailable at the requested portable path")]
    WalUnavailable,
    #[error("backup failed: {0}")]
    Backup(#[from] std::io::Error),
    #[error("released migration {version} has a different checksum")]
    Checksum { version: i64 },
}

pub fn current_version(connection: &Connection) -> Result<i64, MigrationError> {
    let exists: i64 = connection.query_row(
        "SELECT count(*) FROM sqlite_master WHERE type = ?1 AND name = ?2",
        ("table", "schema_migrations"),
        |r| r.get(0),
    )?;
    if exists == 0 {
        return Ok(0);
    }
    Ok(connection.query_row(
        "SELECT COALESCE(MAX(version),0) FROM schema_migrations",
        [],
        |r| r.get(0),
    )?)
}

pub fn migrate(connection: &mut Connection, path: &Path) -> Result<(), MigrationError> {
    let version = current_version(connection)?;
    if version > LATEST_SCHEMA_VERSION {
        return Err(MigrationError::FutureSchema {
            found: version,
            supported: LATEST_SCHEMA_VERSION,
        });
    }
    if version > 0 {
        let checksum: Option<String> = connection
            .query_row(
                "SELECT checksum FROM schema_migrations WHERE version = ?1",
                [1],
                |r| r.get(0),
            )
            .optional()?;
        if checksum.as_deref() != Some(INITIAL_CHECKSUM) {
            return Err(MigrationError::Checksum { version: 1 });
        }
    }
    if version == LATEST_SCHEMA_VERSION {
        return Ok(());
    }
    if version > 0 || database_has_objects(connection)? {
        verified_backup(connection, path)?;
    }
    let transaction = connection.transaction()?;
    transaction.execute_batch("CREATE TABLE IF NOT EXISTS schema_migrations (version INTEGER PRIMARY KEY, identifier TEXT NOT NULL UNIQUE, checksum TEXT NOT NULL, applied_at TEXT NOT NULL);")?;
    if version < 1 {
        transaction.execute_batch(INITIAL_SQL)?;
        transaction.execute("INSERT INTO schema_migrations(version,identifier,checksum,applied_at) VALUES(?1,?2,?3,datetime('now'))", (1_i64, "0001_initial", INITIAL_CHECKSUM))?;
    }
    if version < 2 {
        transaction.execute_batch(RECONCILIATION_SQL)?;
        transaction.execute("INSERT INTO schema_migrations(version,identifier,checksum,applied_at) VALUES(?1,?2,?3,datetime('now'))", (2_i64, "0002_reconciliation_history", "0002-reconciliation-v1"))?;
    }
    transaction.commit()?;
    Ok(())
}

fn database_has_objects(c: &Connection) -> Result<bool, rusqlite::Error> {
    c.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name NOT LIKE ?1)",
        ["sqlite_%"],
        |r| r.get(0),
    )
}

fn verified_backup(source: &Connection, path: &Path) -> Result<PathBuf, MigrationError> {
    let backup_path = path.with_extension("pre-migration.sqlite3");
    if backup_path.exists() {
        fs::remove_file(&backup_path)?;
    }
    let mut destination = Connection::open(&backup_path)?;
    let backup = rusqlite::backup::Backup::new(source, &mut destination)?;
    backup.run_to_completion(64, std::time::Duration::from_millis(10), None)?;
    drop(backup);
    let result: String = destination.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
    if result != "ok" {
        return Err(MigrationError::Backup(std::io::Error::other(
            "backup integrity check failed",
        )));
    }
    Ok(backup_path)
}
