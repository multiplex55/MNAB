use rusqlite::{Connection, OptionalExtension};
use std::{
    fs,
    path::{Path, PathBuf},
};
use thiserror::Error;

pub const LATEST_SCHEMA_VERSION: i64 = 4;
const INITIAL_SQL: &str = include_str!("migrations/0001_initial.sql");
const RECONCILIATION_SQL: &str = include_str!("migrations/0002_reconciliation_history.sql");
const CREDIT_CARD_SQL: &str = include_str!("migrations/0003_credit_card_payment_categories.sql");
const PERSISTENCE_AND_IMPORTS_SQL: &str =
    include_str!("migrations/0004_persistence_and_imports.sql");

struct ReleasedMigration {
    version: i64,
    identifier: &'static str,
    checksum: &'static str,
    sql: &'static str,
}

const RELEASED_MIGRATIONS: &[ReleasedMigration] = &[
    ReleasedMigration {
        version: 1,
        identifier: "0001_initial",
        checksum: "0001-initial-v1",
        sql: INITIAL_SQL,
    },
    ReleasedMigration {
        version: 2,
        identifier: "0002_reconciliation_history",
        checksum: "0002-reconciliation-v1",
        sql: RECONCILIATION_SQL,
    },
    ReleasedMigration {
        version: 3,
        identifier: "0003_credit_card_payment_categories",
        checksum: "0003-credit-card-v1",
        sql: CREDIT_CARD_SQL,
    },
    ReleasedMigration {
        version: 4,
        identifier: "0004_persistence_and_imports",
        checksum: "0004-persistence-imports-v1",
        sql: PERSISTENCE_AND_IMPORTS_SQL,
    },
];

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
    for migration in RELEASED_MIGRATIONS.iter().filter(|m| m.version <= version) {
        let stored: Option<(String, String)> = connection
            .query_row(
                "SELECT identifier,checksum FROM schema_migrations WHERE version = ?1",
                [migration.version],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if stored.as_ref().is_none_or(|(identifier, checksum)| {
            identifier != migration.identifier || checksum != migration.checksum
        }) {
            return Err(MigrationError::Checksum {
                version: migration.version,
            });
        }
    }
    if version == LATEST_SCHEMA_VERSION {
        return Ok(());
    }
    if version > 0 || database_has_objects(connection)? {
        verified_backup(connection, path)?;
    }
    // Table reconstruction cannot run while immediate FK checks still refer to the renamed
    // parent. The migration itself recreates all parents and we validate before re-enabling.
    let reconstructs_tables = version < 4;
    if reconstructs_tables {
        connection.execute_batch("PRAGMA foreign_keys = OFF;")?;
    }
    let result = apply_migrations(connection, version);
    if reconstructs_tables {
        connection.execute_batch("PRAGMA foreign_keys = ON;")?;
    }
    result
}

fn apply_migrations(connection: &mut Connection, version: i64) -> Result<(), MigrationError> {
    let transaction = connection.transaction()?;
    transaction.execute_batch("CREATE TABLE IF NOT EXISTS schema_migrations (version INTEGER PRIMARY KEY, identifier TEXT NOT NULL UNIQUE, checksum TEXT NOT NULL, applied_at TEXT NOT NULL);")?;
    for migration in RELEASED_MIGRATIONS.iter().filter(|m| m.version > version) {
        transaction.execute_batch(migration.sql)?;
        transaction.execute(
            "INSERT INTO schema_migrations(version,identifier,checksum,applied_at) VALUES(?1,?2,?3,datetime('now'))",
            (migration.version, migration.identifier, migration.checksum),
        )?;
    }
    let violations: i64 =
        transaction.query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
            row.get(0)
        })?;
    if violations != 0 {
        return Err(MigrationError::Sqlite(
            rusqlite::Error::ExecuteReturnedResults,
        ));
    }
    transaction.commit()?;
    Ok(())
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::*;

    fn released_database(path: &Path, version: i64) -> Connection {
        let mut connection = Connection::open(path).unwrap();
        connection.execute_batch("PRAGMA foreign_keys=OFF; CREATE TABLE schema_migrations (version INTEGER PRIMARY KEY, identifier TEXT NOT NULL UNIQUE, checksum TEXT NOT NULL, applied_at TEXT NOT NULL);").unwrap();
        let transaction = connection.transaction().unwrap();
        for migration in RELEASED_MIGRATIONS
            .iter()
            .filter(|migration| migration.version <= version)
        {
            transaction.execute_batch(migration.sql).unwrap();
            transaction
                .execute(
                    "INSERT INTO schema_migrations VALUES(?1,?2,?3,'now')",
                    (migration.version, migration.identifier, migration.checksum),
                )
                .unwrap();
        }
        transaction.commit().unwrap();
        connection.execute_batch("PRAGMA foreign_keys=ON").unwrap();
        connection
    }

    #[test]
    fn upgrades_every_released_version_reopens_idempotently_and_backs_up() {
        for version in 1..LATEST_SCHEMA_VERSION {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join(format!("v{version}.sqlite3"));
            let mut connection = released_database(&path, version);
            migrate(&mut connection, &path).unwrap();
            assert_eq!(current_version(&connection).unwrap(), LATEST_SCHEMA_VERSION);
            assert!(path.with_extension("pre-migration.sqlite3").exists());
            migrate(&mut connection, &path).unwrap();
            assert_eq!(current_version(&connection).unwrap(), LATEST_SCHEMA_VERSION);
        }
    }

    #[test]
    fn rejects_changed_checksums_and_future_schemas() {
        let directory = tempfile::tempdir().unwrap();
        let checksum_path = directory.path().join("checksum.sqlite3");
        let mut checksum = released_database(&checksum_path, 3);
        checksum
            .execute(
                "UPDATE schema_migrations SET checksum='changed' WHERE version=2",
                [],
            )
            .unwrap();
        assert!(matches!(
            migrate(&mut checksum, &checksum_path),
            Err(MigrationError::Checksum { version: 2 })
        ));

        let future_path = directory.path().join("future.sqlite3");
        let mut future = released_database(&future_path, 4);
        future
            .execute(
                "INSERT INTO schema_migrations VALUES(99,'future','future','now')",
                [],
            )
            .unwrap();
        assert!(matches!(
            migrate(&mut future, &future_path),
            Err(MigrationError::FutureSchema { found: 99, .. })
        ));
    }

    #[test]
    fn new_defaults_constraints_indexes_and_fields_round_trip() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("roundtrip.sqlite3");
        let mut db = Connection::open(&path).unwrap();
        db.execute_batch("PRAGMA foreign_keys=ON").unwrap();
        migrate(&mut db, &path).unwrap();
        db.execute_batch("INSERT INTO budgets VALUES('b','Budget','now','now',0); INSERT INTO category_groups VALUES('g','b','Group',0,0); INSERT INTO categories VALUES('c','b','g','Category',0,0,0); INSERT INTO accounts(id,budget_id,name,account_type,sort_order,note,favorite,created_at,modified_at) VALUES('a','b','Investments','investment',0,'long term',1,'now','now'); INSERT INTO payees(id,budget_id,name,archived,hidden,default_category_id,last_used_category_id) VALUES('p','b','Payee',0,1,'c','c'); INSERT INTO transactions(id,budget_id,account_id,transaction_date,amount,cleared_state,approval_state,created_at,modified_at,voided) VALUES('t','b','a','2026-01-02',10,'uncleared','approved','now','now',1); INSERT INTO scheduled_transactions(id,budget_id,account_id,start_date,recurrence,custom_interval_days,end_date,amount,sort_order) VALUES('s','b','a','2026-01-01','custom_days',3,'2026-02-01',10,0); INSERT INTO scheduled_occurrences(id,budget_id,schedule_id,sequence,occurrence_date,amount,disposition,transaction_id) VALUES('o','b','s',0,'2026-01-01',10,'entered','t'); INSERT INTO import_batches VALUES('batch','b','a','file','now','staged'); INSERT INTO import_sources(id,budget_id,account_id,source_identifier,archive_status,archive_retry_count,created_at,modified_at) VALUES('src','b','a','bank','archive_failed',2,'now','now'); INSERT INTO import_identities VALUES('identity','b','a','t','src','row-1','fit-1','fingerprint','now'); INSERT INTO staged_import_candidates(id,budget_id,batch_id,source_id,source_record_id,normalized_fingerprint,transaction_date,amount,sort_order) VALUES('candidate','b','batch','src','row-1','fingerprint','2026-01-02',10,0); INSERT INTO import_decisions VALUES('candidate','b','manual_match','t','now'); INSERT INTO import_manual_matches VALUES('candidate','b','t','now');").unwrap();
        let values: (String,i64,String,i64,String,String) = db.query_row("SELECT a.note,a.favorite,p.name,t.voided,s.recurrence,o.disposition FROM accounts a JOIN payees p ON p.id='p' JOIN transactions t ON t.id='t' JOIN scheduled_transactions s ON s.id='s' JOIN scheduled_occurrences o ON o.id='o' WHERE a.id='a'", [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).unwrap();
        assert_eq!(
            values,
            (
                "long term".into(),
                1,
                "Payee".into(),
                1,
                "custom_days".into(),
                "entered".into()
            )
        );
        assert!(db.execute("INSERT INTO scheduled_transactions(id,budget_id,account_id,start_date,recurrence,custom_interval_days,amount,sort_order) VALUES('bad','b','a','2026-01-01','custom_days',0,1,0)", []).is_err());
        for index in [
            "idx_transactions_register_page",
            "idx_import_identity_fingerprint",
            "idx_schedule_occurrence_lookup",
            "idx_assignments_month_category",
        ] {
            assert_eq!(
                db.query_row(
                    "SELECT count(*) FROM sqlite_master WHERE type='index' AND name=?1",
                    [index],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
                1
            );
        }
    }

    #[test]
    fn failed_reconstruction_rolls_back_and_leaves_verified_backup_usable() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("rollback.sqlite3");
        let mut db = released_database(&path, 3);
        db.execute_batch("INSERT INTO budgets VALUES('b','Budget','now','now',0); INSERT INTO accounts VALUES('a','b','Checking','checking',0,0,'now','now');").unwrap();
        let backup = verified_backup(&db, &path).unwrap();
        db.execute_batch("PRAGMA foreign_keys=OFF").unwrap();
        let transaction = db.transaction().unwrap();
        transaction
            .execute_batch(PERSISTENCE_AND_IMPORTS_SQL)
            .unwrap();
        assert!(
            transaction
                .execute_batch("INSERT INTO no_such_table VALUES(1)")
                .is_err()
        );
        drop(transaction);
        assert_eq!(current_version(&db).unwrap(), 3);
        assert_eq!(
            db.query_row("SELECT name FROM accounts WHERE id='a'", [], |r| r
                .get::<_, String>(0))
                .unwrap(),
            "Checking"
        );
        let backup_db = Connection::open(backup).unwrap();
        assert_eq!(
            backup_db
                .query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))
                .unwrap(),
            "ok"
        );
        assert_eq!(
            backup_db
                .query_row("SELECT name FROM accounts WHERE id='a'", [], |r| r
                    .get::<_, String>(0))
                .unwrap(),
            "Checking"
        );
    }
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
