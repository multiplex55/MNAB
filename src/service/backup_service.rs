//! Consistent, portable `SQLite` backups and deterministic retention.
//!
//! Database files are never copied directly: snapshots are produced by `SQLite`'s online backup
//! API, then independently opened, checked, hashed, and finally published with their metadata.

use std::{
    collections::{HashMap, HashSet},
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use rusqlite::{Connection, OpenFlags, backup::Backup};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use time::{Date, OffsetDateTime};
use uuid::Uuid;

const COPY_PAGES: i32 = 128;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BackupReason {
    Manual,
    PreMigration,
    PreImport,
    PreRestore,
    Shutdown,
    Scheduled,
}

impl BackupReason {
    fn is_manual_or_pre_operation(self) -> bool {
        matches!(
            self,
            Self::Manual | Self::PreMigration | Self::PreImport | Self::PreRestore
        )
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BackupMetadata {
    pub format_version: u32,
    pub budget_id: String,
    pub schema_version: i64,
    pub created_at: OffsetDateTime,
    pub reason: BackupReason,
    pub checksum_sha256: String,
    pub application_version: String,
    pub database_file: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BackupArtifact {
    pub database: PathBuf,
    pub metadata: PathBuf,
    pub details: BackupMetadata,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BackupProgress {
    pub completed_pages: u64,
    pub total_pages: Option<u64>,
    pub stage: BackupStage,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BackupStage {
    Copying,
    Validating,
    Publishing,
}

#[derive(Clone, Debug, Default)]
pub struct BackupCancellation {
    cancelled: Arc<AtomicBool>,
}

impl BackupCancellation {
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum BackupError {
    #[error("backup I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("backup SQLite operation failed: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("backup metadata is invalid: {0}")]
    Metadata(#[from] serde_json::Error),
    #[error("backup failed validation: {0}")]
    Validation(String),
    #[error("backup was cancelled before the critical copy stage completed")]
    Cancelled,
    #[error("backup path is outside of the managed backup location")]
    OutOfScope,
}

pub struct BackupService {
    root: PathBuf,
    application_version: String,
}

impl BackupService {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            application_version: env!("CARGO_PKG_VERSION").to_owned(),
        }
    }

    /// Creates and validates a snapshot before publishing its metadata. A failed attempt never
    /// appears as known-good because the metadata is written only after every check succeeds.
    pub fn create(
        &self,
        source: &Connection,
        budget_id_or_name: &str,
        schema_version: i64,
        reason: BackupReason,
    ) -> Result<BackupArtifact, BackupError> {
        self.create_with_progress(
            source,
            budget_id_or_name,
            schema_version,
            reason,
            &BackupCancellation::default(),
            |_| {},
        )
    }

    pub fn create_with_progress(
        &self,
        source: &Connection,
        budget_id_or_name: &str,
        schema_version: i64,
        reason: BackupReason,
        cancellation: &BackupCancellation,
        mut progress: impl FnMut(BackupProgress),
    ) -> Result<BackupArtifact, BackupError> {
        self.create_at(
            source,
            budget_id_or_name,
            schema_version,
            reason,
            OffsetDateTime::now_utc(),
            cancellation,
            &mut progress,
        )
    }

    fn create_at(
        &self,
        source: &Connection,
        budget_id_or_name: &str,
        schema_version: i64,
        reason: BackupReason,
        created_at: OffsetDateTime,
        cancellation: &BackupCancellation,
        progress: &mut impl FnMut(BackupProgress),
    ) -> Result<BackupArtifact, BackupError> {
        let root = self.managed_root()?;
        let directory = root.join(safe_component(budget_id_or_name));
        fs::create_dir_all(&directory)?;
        let timestamp = created_at
            .format(&time::macros::format_description!(
                "[year][month][day]T[hour][minute][second].[subsecond digits:6]Z"
            ))
            .map_err(|error| BackupError::Validation(error.to_string()))?;
        let stem = format!("{timestamp}-{}", Uuid::new_v4().simple());
        let database = directory.join(format!("{stem}.sqlite3"));
        let partial = directory.join(format!(".{stem}.partial"));
        let metadata = directory.join(format!("{stem}.json"));

        let result = (|| {
            if cancellation.is_cancelled() {
                return Err(BackupError::Cancelled);
            }
            let mut destination = Connection::open_with_flags(
                &partial,
                OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
            )?;
            let page_count = source
                .query_row("PRAGMA page_count", [], |row| row.get::<_, u64>(0))
                .ok();
            progress(BackupProgress {
                completed_pages: 0,
                total_pages: page_count,
                stage: BackupStage::Copying,
            });
            let backup = Backup::new(source, &mut destination)?;
            backup.run_to_completion(COPY_PAGES, Duration::from_millis(5), None)?;
            drop(backup);
            progress(BackupProgress {
                completed_pages: page_count.unwrap_or(0),
                total_pages: page_count,
                stage: BackupStage::Validating,
            });
            validate_connection(&destination)?;
            drop(destination);
            let checksum_sha256 = checksum(&partial)?;
            if cancellation.is_cancelled() {
                return Err(BackupError::Cancelled);
            }
            progress(BackupProgress {
                completed_pages: page_count.unwrap_or(0),
                total_pages: page_count,
                stage: BackupStage::Publishing,
            });
            fs::rename(&partial, &database)?;

            let details = BackupMetadata {
                format_version: 1,
                budget_id: budget_id_or_name.to_owned(),
                schema_version,
                created_at,
                reason,
                checksum_sha256,
                application_version: self.application_version.clone(),
                database_file: database
                    .file_name()
                    .expect("generated backup has a file name")
                    .to_string_lossy()
                    .into_owned(),
            };
            write_metadata_atomically(&metadata, &details)?;
            Ok(BackupArtifact {
                database: database.clone(),
                metadata,
                details,
            })
        })();
        if result.is_err() {
            let _ = fs::remove_file(partial);
            // A database without metadata is deliberately not a known-good backup.
            let _ = fs::remove_file(database);
        }
        result
    }

    pub fn expose_location(&self) -> Result<PathBuf, BackupError> {
        self.managed_root()
    }

    fn managed_root(&self) -> Result<PathBuf, BackupError> {
        Ok(canonicalize_existing_parent(&self.root)?)
    }

    pub fn validate(&self, metadata_path: &Path) -> Result<BackupArtifact, BackupError> {
        let root = self.managed_root()?;
        let metadata_path = canonicalize_existing_parent(metadata_path)?;
        if !metadata_path.starts_with(&root) {
            return Err(BackupError::OutOfScope);
        }
        let details: BackupMetadata = serde_json::from_reader(fs::File::open(&metadata_path)?)?;
        if details.format_version != 1 {
            return Err(BackupError::Validation(format!(
                "unsupported metadata version {}",
                details.format_version
            )));
        }
        let database = metadata_path
            .parent()
            .ok_or_else(|| BackupError::Validation("metadata has no parent".into()))?
            .join(&details.database_file);
        let database = canonicalize_existing_parent(&database)?;
        if !database.starts_with(&root) {
            return Err(BackupError::OutOfScope);
        }
        if checksum(&database)? != details.checksum_sha256 {
            return Err(BackupError::Validation("checksum mismatch".into()));
        }
        let connection = Connection::open_with_flags(&database, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        validate_connection(&connection)?;
        Ok(BackupArtifact {
            database,
            metadata: metadata_path.to_owned(),
            details,
        })
    }

    /// Applies mutually-exclusive retention buckets. The newest valid backup is always retained,
    /// and deletion is disabled until at least one newer known-good generation exists.
    pub fn apply_retention(&self, budget_id_or_name: &str) -> Result<Vec<PathBuf>, BackupError> {
        let directory = self.managed_root()?.join(safe_component(budget_id_or_name));
        if !directory.exists() {
            return Ok(Vec::new());
        }
        let mut valid = Vec::new();
        for entry in fs::read_dir(&directory)? {
            let path = entry?.path();
            if path
                .extension()
                .is_some_and(|extension| extension == "json")
                && let Ok(artifact) = self.validate(&path)
            {
                valid.push(artifact);
            }
        }
        valid.sort_by_key(|artifact| std::cmp::Reverse(artifact.details.created_at));
        if valid.len() < 2 {
            return Ok(Vec::new());
        }

        let newest = valid[0].database.clone();
        let mut keep = HashSet::from([newest]);
        let mut manual_count = 0_usize;
        let retention_now = valid[0].details.created_at;
        let cutoff_daily = retention_now - time::Duration::days(30);
        let cutoff_monthly = retention_now - time::Duration::days(366);
        let mut daily = HashSet::<Date>::new();
        let mut monthly = HashSet::<(i32, time::Month)>::new();

        // Each artifact enters the first matching bucket and can never consume two generations.
        for artifact in &valid {
            if keep.contains(&artifact.database) {
                continue;
            }
            if artifact.details.reason.is_manual_or_pre_operation() && manual_count < 10 {
                manual_count += 1;
                keep.insert(artifact.database.clone());
                continue;
            }
            let date = artifact.details.created_at.date();
            if artifact.details.created_at >= cutoff_daily && daily.insert(date) {
                keep.insert(artifact.database.clone());
                continue;
            }
            let month = (date.year(), date.month());
            if artifact.details.created_at >= cutoff_monthly && monthly.insert(month) {
                keep.insert(artifact.database.clone());
            }
        }

        let metadata_by_database: HashMap<_, _> = valid
            .iter()
            .map(|artifact| (artifact.database.clone(), artifact.metadata.clone()))
            .collect();
        let mut removed = Vec::new();
        for artifact in valid.iter().skip(1) {
            if !keep.contains(&artifact.database) {
                fs::remove_file(&artifact.database)?;
                if let Some(path) = metadata_by_database.get(&artifact.database) {
                    fs::remove_file(path)?;
                }
                removed.push(artifact.database.clone());
            }
        }
        Ok(removed)
    }
}

fn canonicalize_existing_parent(path: &Path) -> Result<PathBuf, std::io::Error> {
    if path.exists() {
        return path.canonicalize();
    }
    let parent = path
        .parent()
        .ok_or_else(|| std::io::Error::other("path has no parent"))?
        .canonicalize()?;
    Ok(parent.join(
        path.file_name()
            .ok_or_else(|| std::io::Error::other("path has no file name"))?,
    ))
}

fn safe_component(value: &str) -> String {
    let safe: String = value
        .chars()
        .map(|character| {
            if character.is_alphanumeric() || matches!(character, '-' | '_' | '.') {
                character
            } else {
                '_'
            }
        })
        .take(80)
        .collect();
    if safe.trim_matches(['.', '_']).is_empty() {
        format!("budget-{}", &hex_digest(value.as_bytes())[..16])
    } else {
        safe
    }
}

fn validate_connection(connection: &Connection) -> Result<(), BackupError> {
    for pragma in ["PRAGMA quick_check", "PRAGMA foreign_key_check"] {
        let mut statement = connection.prepare(pragma)?;
        let mut rows = statement.query([])?;
        if pragma.ends_with("quick_check") {
            let result: String = rows
                .next()?
                .ok_or_else(|| BackupError::Validation("quick_check returned no result".into()))?
                .get(0)?;
            if result != "ok" || rows.next()?.is_some() {
                return Err(BackupError::Validation(format!(
                    "quick_check reported {result}"
                )));
            }
        } else if rows.next()?.is_some() {
            return Err(BackupError::Validation(
                "foreign_key_check reported violations".into(),
            ));
        }
    }
    Ok(())
}

fn checksum(path: &Path) -> Result<String, std::io::Error> {
    let mut file = fs::File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn hex_digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn write_metadata_atomically(path: &Path, metadata: &BackupMetadata) -> Result<(), BackupError> {
    let temporary = path.with_extension("json.partial");
    let bytes = serde_json::to_vec_pretty(metadata)?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    drop(file);
    fs::rename(temporary, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unicode_and_simultaneous_names_are_safe_and_unique() {
        let directory = tempfile::tempdir().unwrap();
        let source = Connection::open_in_memory().unwrap();
        source
            .execute_batch("CREATE TABLE values_(value TEXT); INSERT INTO values_ VALUES('ok');")
            .unwrap();
        let service = BackupService::new(directory.path());
        let first = service
            .create(&source, "家計 / 2026", 3, BackupReason::Manual)
            .unwrap();
        let second = service
            .create(&source, "家計 / 2026", 3, BackupReason::Manual)
            .unwrap();
        assert_ne!(first.database, second.database);
        assert_eq!(
            service.validate(&first.metadata).unwrap().details.budget_id,
            "家計 / 2026"
        );
    }

    #[test]
    fn metadata_is_not_published_when_validation_fails() {
        let directory = tempfile::tempdir().unwrap();
        let source = Connection::open_in_memory().unwrap();
        source.execute_batch("PRAGMA foreign_keys=OFF; CREATE TABLE p(id PRIMARY KEY); CREATE TABLE c(p REFERENCES p); INSERT INTO c VALUES(9);").unwrap();
        let service = BackupService::new(directory.path());
        assert!(
            service
                .create(&source, "broken", 3, BackupReason::Manual)
                .is_err()
        );
        let backup_dir = directory.path().join("broken");
        assert_eq!(fs::read_dir(backup_dir).unwrap().count(), 0);
    }
}
