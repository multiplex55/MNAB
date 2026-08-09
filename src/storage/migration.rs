use rusqlite::{Connection, OptionalExtension};
use std::{
    fs,
    path::{Path, PathBuf},
};
use thiserror::Error;

pub const LATEST_SCHEMA_VERSION: i64 = 2;
pub const SCHEMA_FAMILY_KEY: &str = "schema_family";
pub const SCHEMA_FAMILY: &str = "account_centric_v1";
const ACCOUNT_CENTRIC_SCHEMA_SQL: &str = include_str!("migrations/0001_account_centric_schema.sql");
const TRANSACTION_RULE_SQL: &str = include_str!("migrations/0002_transaction_rules.sql");

struct ReleasedMigration {
    version: i64,
    identifier: &'static str,
    checksum: &'static str,
    sql: &'static str,
}

const RELEASED_MIGRATIONS: &[ReleasedMigration] = &[
    ReleasedMigration {
        version: 1,
        identifier: "0001_account_centric_schema",
        checksum: "0001-account-centric-schema-v1",
        sql: ACCOUNT_CENTRIC_SCHEMA_SQL,
    },
    ReleasedMigration {
        version: 2,
        identifier: "0002_transaction_rules",
        checksum: "0002-transaction-rules-v1",
        sql: TRANSACTION_RULE_SQL,
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
    #[error("database schema family {found:?} is not supported; expected account_centric_v1")]
    SchemaFamily { found: Option<String> },
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
    validate_schema_family(connection, version)?;
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
    apply_migrations(connection, version)
}

fn validate_schema_family(connection: &Connection, version: i64) -> Result<(), MigrationError> {
    let metadata_exists: i64 = connection.query_row(
        "SELECT count(*) FROM sqlite_master WHERE type = ?1 AND name = ?2",
        ("table", "application_metadata"),
        |r| r.get(0),
    )?;
    if metadata_exists == 0 {
        if version == 0 && !database_has_objects(connection)? {
            return Ok(());
        }
        return Err(MigrationError::SchemaFamily { found: None });
    }
    let family: Option<String> = connection
        .query_row(
            "SELECT value FROM application_metadata WHERE key = ?1",
            [SCHEMA_FAMILY_KEY],
            |r| r.get(0),
        )
        .optional()?;
    if family.as_deref() == Some(SCHEMA_FAMILY) {
        Ok(())
    } else {
        Err(MigrationError::SchemaFamily { found: family })
    }
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
        transaction.execute(
            "INSERT INTO application_metadata(key,value,updated_at) VALUES(?1,?2,datetime('now')) ON CONFLICT(key) DO UPDATE SET value=excluded.value,updated_at=excluded.updated_at",
            (SCHEMA_FAMILY_KEY, SCHEMA_FAMILY),
        )?;
        transaction.execute(
            "INSERT INTO application_metadata(key,value,updated_at) VALUES('schema_version',?1,datetime('now')) ON CONFLICT(key) DO UPDATE SET value=excluded.value,updated_at=excluded.updated_at",
            [migration.version.to_string()],
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
        if version > 0 {
            transaction
                .execute(
                    "INSERT INTO application_metadata(key,value,updated_at) VALUES(?1,?2,'now')",
                    (SCHEMA_FAMILY_KEY, SCHEMA_FAMILY),
                )
                .unwrap();
            transaction
                .execute(
                    "INSERT INTO application_metadata(key,value,updated_at) VALUES('schema_version',?1,'now')",
                    [version.to_string()],
                )
                .unwrap();
        }
        transaction.commit().unwrap();
        connection.execute_batch("PRAGMA foreign_keys=ON").unwrap();
        connection
    }

    #[test]
    fn creates_fresh_account_centric_schema_and_reopens_idempotently() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("mnab.sqlite3");
        let mut connection = Connection::open(&path).unwrap();

        migrate(&mut connection, &path).unwrap();
        assert_eq!(current_version(&connection).unwrap(), LATEST_SCHEMA_VERSION);
        assert_eq!(
            connection
                .query_row(
                    "SELECT value FROM application_metadata WHERE key=?1",
                    [SCHEMA_FAMILY_KEY],
                    |r| r.get::<_, String>(0)
                )
                .unwrap(),
            SCHEMA_FAMILY
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT value FROM application_metadata WHERE key='schema_version'",
                    [],
                    |r| r.get::<_, String>(0)
                )
                .unwrap(),
            LATEST_SCHEMA_VERSION.to_string()
        );
        migrate(&mut connection, &path).unwrap();
        assert_eq!(current_version(&connection).unwrap(), LATEST_SCHEMA_VERSION);
    }

    #[test]
    fn rejects_changed_checksums_and_future_schemas() {
        let directory = tempfile::tempdir().unwrap();
        let checksum_path = directory.path().join("checksum.sqlite3");
        let mut checksum = released_database(&checksum_path, LATEST_SCHEMA_VERSION);
        checksum
            .execute(
                "UPDATE schema_migrations SET checksum='changed' WHERE version=1",
                [],
            )
            .unwrap();
        assert!(matches!(
            migrate(&mut checksum, &checksum_path),
            Err(MigrationError::Checksum { version: 1 })
        ));

        let future_path = directory.path().join("future.sqlite3");
        let mut future = released_database(&future_path, LATEST_SCHEMA_VERSION);
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
    fn migration_registry_matches_active_sql_files() {
        let mut versions = std::collections::BTreeSet::new();
        let active_files =
            fs::read_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/storage/migrations"))
                .unwrap()
                .filter_map(|entry| {
                    let entry = entry.unwrap();
                    let path = entry.path();
                    (path.extension().and_then(|value| value.to_str()) == Some("sql"))
                        .then_some(path)
                })
                .collect::<Vec<_>>();

        assert_eq!(active_files.len(), RELEASED_MIGRATIONS.len());
        for migration in RELEASED_MIGRATIONS {
            assert!(
                versions.insert(migration.version),
                "duplicate migration version {}",
                migration.version
            );
            let expected = format!(
                "{:04}_{}.sql",
                migration.version,
                migration
                    .identifier
                    .trim_start_matches(&format!("{:04}_", migration.version))
            );
            assert!(
                active_files
                    .iter()
                    .any(|path| path.file_name().unwrap() == expected.as_str()),
                "missing active SQL file for registered migration {}",
                migration.identifier
            );
        }
        for (expected, actual) in (1..=LATEST_SCHEMA_VERSION).zip(versions.iter().copied()) {
            assert_eq!(actual, expected, "skipped migration version {expected}");
        }
        assert_eq!(versions.last().copied(), Some(LATEST_SCHEMA_VERSION));

        for path in active_files {
            let file_name = path.file_stem().unwrap().to_str().unwrap();
            let (version, _) = file_name.split_once('_').unwrap();
            let version = version.parse::<i64>().unwrap();
            let migration = RELEASED_MIGRATIONS
                .iter()
                .find(|migration| migration.version == version);
            assert_eq!(
                migration.map(|migration| migration.identifier),
                Some(file_name),
                "unregistered or mismatched migration file {}",
                path.display()
            );
        }
    }

    #[test]
    fn rejects_non_account_centric_schema_family() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("mnab.sqlite3");
        let mut db = Connection::open(&path).unwrap();
        db.execute_batch("CREATE TABLE application_metadata(key TEXT PRIMARY KEY, value TEXT NOT NULL, updated_at TEXT NOT NULL); INSERT INTO application_metadata VALUES('schema_family','budget_first_v1','now');").unwrap();

        assert!(matches!(
            migrate(&mut db, &path),
            Err(MigrationError::SchemaFamily { found: Some(family) }) if family == "budget_first_v1"
        ));
    }

    #[test]
    fn fresh_account_centric_schema_passes_sqlite_checks() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("mnab.sqlite3");
        let mut db = Connection::open(&path).unwrap();
        db.execute_batch("PRAGMA foreign_keys=ON").unwrap();
        migrate(&mut db, &path).unwrap();

        let fk_violations: i64 = db
            .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(fk_violations, 0);
        assert_eq!(
            db.query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))
                .unwrap(),
            "ok"
        );
    }

    #[test]
    fn new_defaults_constraints_indexes_and_fields_round_trip() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("roundtrip.sqlite3");
        let mut db = Connection::open(&path).unwrap();
        db.execute_batch("PRAGMA foreign_keys=ON").unwrap();
        migrate(&mut db, &path).unwrap();
        db.execute_batch("INSERT INTO budgets VALUES('b','Budget','now','now',0); INSERT INTO category_groups VALUES('g','b','Group',0,0); INSERT INTO categories VALUES('c','b','g','Category',0,0,0); INSERT INTO accounts(id,budget_id,name,account_type,sort_order,note,favorite,created_at,modified_at) VALUES('a','b','Liability','liability',0,'long term',1,'now','now'); INSERT INTO payees(id,budget_id,name,archived,hidden,default_category_id,last_used_category_id) VALUES('p','b','Payee',0,1,'c','c'); INSERT INTO transactions(id,budget_id,account_id,transaction_date,amount,cleared_state,approval_state,created_at,modified_at,voided) VALUES('t','b','a','2026-01-02',10,'uncleared','approved','now','now',1); INSERT INTO scheduled_transactions(id,budget_id,account_id,start_date,recurrence,custom_interval_days,end_date,amount,sort_order) VALUES('s','b','a','2026-01-01','custom_days',3,'2026-02-01',10,0); INSERT INTO scheduled_occurrences(id,budget_id,schedule_id,sequence,occurrence_date,amount,disposition,transaction_id) VALUES('o','b','s',0,'2026-01-01',10,'entered','t'); INSERT INTO import_batches VALUES('batch','b','a','file','now','staged'); INSERT INTO import_sources(id,budget_id,account_id,source_identifier,archive_status,archive_retry_count,created_at,modified_at) VALUES('src','b','a','bank','archive_failed',2,'now','now'); INSERT INTO import_identities VALUES('identity','b','a','t','src','row-1','fit-1','fingerprint','now'); INSERT INTO staged_import_candidates(id,budget_id,batch_id,source_id,source_record_id,normalized_fingerprint,transaction_date,amount,sort_order) VALUES('candidate','b','batch','src','row-1','fingerprint','2026-01-02',10,0); INSERT INTO import_decisions VALUES('candidate','b','manual_match','t','now'); INSERT INTO import_manual_matches VALUES('candidate','b','t','now');").unwrap();
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
            "idx_transactions_budget_register_page",
            "idx_transactions_budget_report_v7",
            "idx_reconciliations_budget_date",
            "idx_scheduled_occurrences_inbox",
            "idx_staged_candidates_batch_review",
            "idx_change_log_report_revision",
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
    fn verified_backup_copies_current_database() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("backup.sqlite3");
        let db = released_database(&path, LATEST_SCHEMA_VERSION);
        db.execute_batch("INSERT INTO budgets VALUES('b','Budget','now','now',0); INSERT INTO accounts(id,budget_id,name,account_type,sort_order,created_at,modified_at) VALUES('a','b','Checking','checking',0,'now','now');").unwrap();

        let backup = verified_backup(&db, &path).unwrap();
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
