//! Lifecycle management for the fixed portable MNAB database.
//!
//! Active workflows target only `mnab-data/mnab.sqlite3`; legacy files under
//! `mnab-data/budgets` are intentionally ignored and never converted.

use std::{
    fs::{self, OpenOptions},
    io::Read,
    path::{Component, Path, PathBuf},
};

use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    app::{
        portable_paths::PortablePaths,
        session::{BudgetSession, SessionSummary},
    },
    domain::{Budget, BudgetId, BudgetMonth},
    service::budget_service::{
        self, BudgetCreationTransaction, BudgetStorage, CreateBudget, CreateBudgetError,
    },
    storage::{
        diagnostics,
        migration::{self, LATEST_SCHEMA_VERSION},
        worker::StorageWorker,
    },
};

const SQLITE_HEADER: &[u8; 16] = b"SQLite format 3\0";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileExistence {
    Present,
    Missing,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DatabaseRecognition {
    Mnab,
    NotSqlite,
    NotMnab,
    Unreadable,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ValidationResult {
    pub checked_at: OffsetDateTime,
    pub valid: bool,
    pub details: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BudgetCatalogEntry {
    pub budget_id: BudgetId,
    pub display_name: String,
    pub database_path: PathBuf,
    pub schema_version: i64,
    pub last_successful_open: Option<OffsetDateTime>,
    pub last_validation: Option<ValidationResult>,
    pub file_existence: FileExistence,
    pub recognition: DatabaseRecognition,
}

#[derive(Debug, thiserror::Error)]
pub enum CatalogError {
    #[error("path is not a regular managed budget file")]
    UnmanagedPath,
    #[error("not an MNAB database: {0}")]
    NotMnab(String),
    #[error("database schema {found} is newer than supported schema {supported}")]
    FutureSchema { found: i64, supported: i64 },
    #[error("budget name must not be empty")]
    EmptyName,
    #[error("budget was not found in the catalog")]
    NotFound,
    #[error("filesystem operation failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("database operation failed: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("migration failed: {0}")]
    Migration(#[from] migration::MigrationError),
    #[error("creation failed: {0}")]
    Creation(#[from] CreateBudgetError),
}

#[derive(Default, Debug)]
pub struct BudgetCatalog {
    entries: Vec<BudgetCatalogEntry>,
}

impl BudgetCatalog {
    #[must_use]
    pub fn entries(&self) -> &[BudgetCatalogEntry] {
        &self.entries
    }
    /// Refreshes filesystem facts for the fixed database only.
    pub fn refresh(&mut self, paths: &PortablePaths) -> Result<(), CatalogError> {
        self.entries.clear();
        let path = fixed_database(paths);
        if !path.is_file() {
            return Ok(());
        }
        let Some(info) = inspect(&path)? else {
            return Ok(());
        };
        self.entries.push(BudgetCatalogEntry {
            budget_id: info.id,
            display_name: info.name,
            database_path: fs::canonicalize(path)?,
            schema_version: info.version,
            last_successful_open: None,
            last_validation: None,
            file_existence: FileExistence::Present,
            recognition: DatabaseRecognition::Mnab,
        });
        Ok(())
    }

    pub fn rename(
        &mut self,
        paths: &PortablePaths,
        id: BudgetId,
        display_name: &str,
    ) -> Result<(), CatalogError> {
        let name = display_name.trim();
        if name.is_empty() {
            return Err(CatalogError::EmptyName);
        }
        let entry = self.entry_mut(id)?;
        let path = fixed_database(paths);
        let path = fs::canonicalize(&path).map_err(|_| CatalogError::UnmanagedPath)?;
        Connection::open(path)?.execute(
            "UPDATE budgets SET name=?1, modified_at=datetime('now') WHERE id=?2",
            (name, id.to_string()),
        )?;
        entry.display_name = name.into();
        Ok(())
    }

    #[allow(clippy::unused_self)] // Kept as a catalog operation for a cohesive lifecycle API.
    pub fn import_external(
        &self,
        paths: &PortablePaths,
        source: &Path,
    ) -> Result<PathBuf, CatalogError> {
        if !source.is_file() {
            return Err(CatalogError::UnmanagedPath);
        }
        inspect(source)?.ok_or_else(|| CatalogError::NotMnab("unrecognized schema".into()))?;
        let stem = source
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Imported Budget");
        let ext = source
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("sqlite3");
        for suffix in 0_u32.. {
            let name = if suffix == 0 {
                format!("{stem}.{ext}")
            } else {
                format!("{stem} ({suffix}).{ext}")
            };
            budget_service::validate_filename(&name)?;
            let target = paths.budgets.join(name);
            match copy_new(source, &target) {
                Ok(()) => return Ok(fs::canonicalize(target)?),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(e.into()),
            }
        }
        unreachable!()
    }

    #[allow(clippy::unused_self)] // Preparation is intentionally invoked through the catalog.
    pub fn prepare_fixed(
        &self,
        paths: &PortablePaths,
        repaint: impl Fn() + Send + 'static,
    ) -> Result<PreparedBudget, CatalogError> {
        self.prepare_fixed_checked(paths, false, repaint)
    }

    #[allow(clippy::unused_self)] // Kept as a catalog operation for one lifecycle API.
    pub fn prepare_fixed_checked(
        &self,
        paths: &PortablePaths,
        thorough: bool,
        repaint: impl Fn() + Send + 'static,
    ) -> Result<PreparedBudget, CatalogError> {
        let path = fixed_database(paths);
        let path = fs::canonicalize(&path).map_err(|_| CatalogError::UnmanagedPath)?; // 1: fixed path
        let info = inspect(&path)?
            .ok_or_else(|| CatalogError::NotMnab("header or schema not recognized".into()))?; // 2
        let validation = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY)?; // 3
        if info.version > LATEST_SCHEMA_VERSION {
            return Err(CatalogError::FutureSchema {
                found: info.version,
                supported: LATEST_SCHEMA_VERSION,
            });
        } // 4
        let findings = diagnostics::all(&validation, thorough)?; // 5
        if findings
            .iter()
            .any(|f| f.severity == diagnostics::Severity::Error)
        {
            return Err(CatalogError::NotMnab("database diagnostics failed".into()));
        }
        drop(validation);
        let worker = StorageWorker::start(&path, repaint)?; // 6 migrations, then worker startup
        let connection = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let (name, count): (String, i64) = connection.query_row("SELECT b.name,(SELECT count(*) FROM accounts WHERE budget_id=b.id) FROM budgets b WHERE b.id=?1", [info.id.to_string()], |r| Ok((r.get(0)?,r.get(1)?)))?; // 7
        Ok(PreparedBudget {
            session: BudgetSession {
                budget_id: info.id,
                database_path: path,
                schema_version: u32::try_from(LATEST_SCHEMA_VERSION)
                    .expect("schema version fits the session model"),
                summary: SessionSummary {
                    budget_name: name,
                    account_count: usize::try_from(count).unwrap_or_default(),
                },
            },
            worker,
        })
    }

    /// Catalog update is deliberately separate and follows runtime session commit.
    pub fn record_successful_open(&mut self, session: &BudgetSession) {
        if let Some(e) = self
            .entries
            .iter_mut()
            .find(|e| e.budget_id == session.budget_id)
        {
            e.last_successful_open = Some(OffsetDateTime::now_utc());
            e.schema_version = i64::from(session.schema_version);
            e.file_existence = FileExistence::Present;
        }
    }

    /// Resolves an Explorer target from catalog identity rather than accepting an
    /// arbitrary path from the UI.
    pub fn reveal(&self, paths: &PortablePaths, id: BudgetId) -> Result<(), CatalogError> {
        let entry = self
            .entries
            .iter()
            .find(|e| e.budget_id == id)
            .ok_or(CatalogError::NotFound)?;
        let _ = entry;
        reveal_in_explorer(&paths.data)?;
        Ok(())
    }

    /// Repair is restricted to an explicit catalog identity and managed file.
    pub fn repair(
        &self,
        paths: &PortablePaths,
        id: BudgetId,
        request: crate::storage::repair::RepairRequest,
    ) -> Result<crate::storage::repair::RepairReport, CatalogError> {
        let entry = self
            .entries
            .iter()
            .find(|e| e.budget_id == id)
            .ok_or(CatalogError::NotFound)?;
        let _ = entry;
        let path = fixed_database(paths);
        let path = fs::canonicalize(&path).map_err(|_| CatalogError::UnmanagedPath)?;
        crate::storage::repair::repair(&path, request)
            .map_err(|error| CatalogError::NotMnab(format!("repair was not applied: {error}")))
    }

    fn entry_mut(&mut self, id: BudgetId) -> Result<&mut BudgetCatalogEntry, CatalogError> {
        self.entries
            .iter_mut()
            .find(|e| e.budget_id == id)
            .ok_or(CatalogError::NotFound)
    }
}

pub struct PreparedBudget {
    pub session: BudgetSession,
    pub worker: StorageWorker,
}
struct Inspection {
    id: BudgetId,
    name: String,
    version: i64,
}

fn managed_file(root: &Path, candidate: &Path) -> Result<PathBuf, CatalogError> {
    if candidate
        .components()
        .any(|c| matches!(c, Component::ParentDir))
    {
        return Err(CatalogError::UnmanagedPath);
    }
    let canonical = fs::canonicalize(candidate).map_err(|_| CatalogError::UnmanagedPath)?;
    if !canonical.starts_with(root) || !canonical.is_file() {
        return Err(CatalogError::UnmanagedPath);
    }
    Ok(canonical)
}
fn inspect(path: &Path) -> Result<Option<Inspection>, CatalogError> {
    let mut f = fs::File::open(path)?;
    let mut h = [0; 16];
    if f.read_exact(&mut h).is_err() || &h != SQLITE_HEADER {
        return Ok(None);
    }
    let c = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let version = migration::current_version(&c)?;
    let row: Option<(String, String)> = c
        .query_row("SELECT id,name FROM budgets LIMIT 1", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .optional()?;
    let Some((id, name)) = row else {
        return Ok(None);
    };
    let Ok(uuid) = Uuid::parse_str(&id) else {
        return Ok(None);
    };
    Ok(Some(Inspection {
        id: BudgetId::from_uuid(uuid),
        name,
        version,
    }))
}
fn copy_new(source: &Path, target: &Path) -> std::io::Result<()> {
    let mut input = fs::File::open(source)?;
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(target)?;
    std::io::copy(&mut input, &mut output)?;
    output.sync_all()
}

#[cfg(windows)]
pub fn reveal_in_explorer(path: &Path) -> std::io::Result<()> {
    std::process::Command::new("explorer.exe")
        .arg(format!("/select,{}", path.display()))
        .spawn()?
        .wait()?;
    Ok(())
}
#[cfg(not(windows))]
pub fn reveal_in_explorer(_path: &Path) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "Explorer reveal is only available on Windows",
    ))
}

#[must_use]
pub fn fixed_database(paths: &PortablePaths) -> PathBuf {
    paths.database.clone()
}

pub fn validate_fixed(
    paths: &PortablePaths,
    thorough: bool,
) -> Result<Vec<diagnostics::Finding>, CatalogError> {
    let path = fs::canonicalize(fixed_database(paths)).map_err(|_| CatalogError::UnmanagedPath)?;
    let info = inspect(&path)?
        .ok_or_else(|| CatalogError::NotMnab("header or schema not recognized".into()))?;
    if info.version > LATEST_SCHEMA_VERSION {
        return Err(CatalogError::FutureSchema {
            found: info.version,
            supported: LATEST_SCHEMA_VERSION,
        });
    }
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    diagnostics::all(&connection, thorough).map_err(CatalogError::Sqlite)
}

pub fn backup_fixed(
    paths: &PortablePaths,
    reason: crate::service::backup_service::BackupReason,
) -> Result<
    crate::service::backup_service::BackupArtifact,
    crate::service::backup_service::BackupError,
> {
    let path = fixed_database(paths);
    let connection = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let info = inspect(&path)
        .map_err(|e| crate::service::backup_service::BackupError::Validation(e.to_string()))?
        .ok_or_else(|| {
            crate::service::backup_service::BackupError::Validation(
                "fixed database is not an MNAB database".into(),
            )
        })?;
    crate::service::backup_service::BackupService::new(&paths.backups).create(
        &connection,
        &info.id.to_string(),
        info.version,
        reason,
    )
}

pub struct PortableBudgetStorage<'a> {
    root: &'a Path,
}
impl<'a> PortableBudgetStorage<'a> {
    #[must_use]
    pub const fn new(root: &'a Path) -> Self {
        Self { root }
    }
}
pub struct CreationTx {
    final_path: PathBuf,
    temp_path: PathBuf,
    connection: Option<Connection>,
}
impl BudgetStorage for PortableBudgetStorage<'_> {
    type Transaction = CreationTx;
    fn exists(&self, n: &str) -> Result<bool, CreateBudgetError> {
        Ok(self.root.join(n).exists())
    }
    fn begin_create(&mut self, n: &str) -> Result<CreationTx, CreateBudgetError> {
        fs::create_dir_all(self.root).map_err(|e| CreateBudgetError::Storage(e.to_string()))?;
        let final_path = self.root.join(n);
        let temp_path = self.root.join(format!(".create-{}", Uuid::new_v4()));
        let connection = crate::storage::connection::open_primary(&temp_path)
            .map_err(|e| CreateBudgetError::Storage(e.to_string()))?;
        Ok(CreationTx {
            final_path,
            temp_path,
            connection: Some(connection),
        })
    }
}
impl BudgetCreationTransaction for CreationTx {
    fn create_database(
        &mut self,
        b: &Budget,
        _: BudgetMonth,
        _: &str,
    ) -> Result<(), CreateBudgetError> {
        let now = OffsetDateTime::now_utc().to_string();
        self.connection
            .as_ref()
            .unwrap()
            .execute(
                "INSERT INTO budgets(id,name,created_at,modified_at)VALUES(?1,?2,?3,?3)",
                (b.id.to_string(), &b.name, now),
            )
            .map_err(|e| CreateBudgetError::Storage(e.to_string()))?;
        Ok(())
    }
    fn create_group(&mut self, n: &str, p: u32) -> Result<(), CreateBudgetError> {
        let c = self.connection.as_ref().unwrap();
        let bid: String = c
            .query_row("SELECT id FROM budgets LIMIT 1", [], |r| r.get(0))
            .map_err(|e| CreateBudgetError::Storage(e.to_string()))?;
        c.execute(
            "INSERT INTO category_groups(id,budget_id,name,sort_order)VALUES(?1,?2,?3,?4)",
            (Uuid::new_v4().to_string(), bid, n, p),
        )
        .map_err(|e| CreateBudgetError::Storage(e.to_string()))?;
        Ok(())
    }
    fn create_category(&mut self, g: &str, n: &str, p: u32) -> Result<(), CreateBudgetError> {
        let c = self.connection.as_ref().unwrap();
        let (bid, gid): (String, String) = c
            .query_row(
                "SELECT budget_id,id FROM category_groups WHERE name=?1",
                [g],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|e| CreateBudgetError::Storage(e.to_string()))?;
        c.execute(
            "INSERT INTO categories(id,budget_id,group_id,name,sort_order)VALUES(?1,?2,?3,?4,?5)",
            (Uuid::new_v4().to_string(), bid, gid, n, p),
        )
        .map_err(|e| CreateBudgetError::Storage(e.to_string()))?;
        Ok(())
    }
    fn commit(mut self) -> Result<(), CreateBudgetError> {
        let c = self.connection.take().unwrap();
        c.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
            .map_err(|e| CreateBudgetError::Storage(e.to_string()))?;
        drop(c);
        // A hard link is a same-volume, no-replace publication. Readers see the
        // complete database or no database, and a racing creator cannot be overwritten.
        fs::hard_link(&self.temp_path, &self.final_path)
            .map_err(|e| CreateBudgetError::Storage(e.to_string()))?;
        fs::remove_file(&self.temp_path).map_err(|e| CreateBudgetError::Storage(e.to_string()))?;
        Ok(())
    }
}
impl Drop for CreationTx {
    fn drop(&mut self) {
        if let Some(c) = self.connection.take() {
            drop(c);
        }
        let _ = fs::remove_file(&self.temp_path);
        let _ = fs::remove_file(format!("{}-wal", self.temp_path.display()));
        let _ = fs::remove_file(format!("{}-shm", self.temp_path.display()));
    }
}

pub fn create_managed(
    paths: &PortablePaths,
    request: CreateBudget,
) -> Result<Budget, CreateBudgetError> {
    let mut request = request;
    request.database_filename = "mnab.sqlite3".into();
    let mut storage = PortableBudgetStorage::new(&paths.data);
    budget_service::create_budget(&mut storage, request)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::backup_service::BackupReason;

    fn paths() -> (tempfile::TempDir, PortablePaths) {
        let dir = tempfile::tempdir().unwrap();
        let paths = PortablePaths::from_executable(&dir.path().join("mnab.exe")).unwrap();
        (dir, paths)
    }
    fn request(name: &str) -> CreateBudget {
        CreateBudget {
            name: name.into(),
            database_filename: "ignored.sqlite3".into(),
            initial_month: BudgetMonth::new(2026, 8).unwrap(),
            currency: "USD".into(),
            starter_content: true,
        }
    }

    #[test]
    fn fresh_lifecycle_creates_and_opens_only_fixed_database() {
        let (_dir, paths) = paths();
        let budget = create_managed(&paths, request("Household")).unwrap();
        assert!(paths.database.is_file());
        assert_eq!(paths.database.file_name().unwrap(), "mnab.sqlite3");
        assert_eq!(fs::read_dir(&paths.budgets).unwrap().count(), 0);
        let mut catalog = BudgetCatalog::default();
        catalog.refresh(&paths).unwrap();
        assert_eq!(catalog.entries().len(), 1);
        assert_eq!(catalog.entries()[0].budget_id, budget.id);
        assert_eq!(
            catalog.entries()[0].database_path,
            fs::canonicalize(&paths.database).unwrap()
        );
    }

    #[test]
    fn legacy_budget_directory_files_are_ignored_and_untouched() {
        let (_legacy_dir, legacy_paths) = paths();
        create_managed(&legacy_paths, request("Legacy")).unwrap();
        let legacy_bytes = fs::read(&legacy_paths.database).unwrap();
        let (_dir, paths) = paths();
        let legacy = paths.budgets.join("legacy.sqlite3");
        fs::write(&legacy, &legacy_bytes).unwrap();
        let before = fs::metadata(&legacy).unwrap().modified().unwrap();
        let mut catalog = BudgetCatalog::default();
        catalog.refresh(&paths).unwrap();
        assert!(catalog.entries().is_empty());
        assert_eq!(fs::read(&legacy).unwrap(), legacy_bytes);
        assert_eq!(fs::metadata(&legacy).unwrap().modified().unwrap(), before);
        assert!(!paths.database.exists());
    }

    #[test]
    fn renaming_budget_changes_metadata_not_fixed_filename() {
        let (_dir, paths) = paths();
        let budget = create_managed(&paths, request("Before")).unwrap();
        let fixed = paths.database.clone();
        let mut catalog = BudgetCatalog::default();
        catalog.refresh(&paths).unwrap();
        catalog.rename(&paths, budget.id, "After").unwrap();
        assert!(fixed.is_file());
        assert!(!paths.data.join("After.sqlite3").exists());
        assert_eq!(
            catalog.entries()[0].database_path,
            fs::canonicalize(&fixed).unwrap()
        );
        let connection = Connection::open(&fixed).unwrap();
        let name: String = connection
            .query_row("SELECT name FROM budgets", [], |r| r.get(0))
            .unwrap();
        assert_eq!(name, "After");
    }

    #[test]
    fn backup_validate_and_repair_target_fixed_database() {
        let (_dir, paths) = paths();
        create_managed(&paths, request("Maintained")).unwrap();
        assert!(
            validate_fixed(&paths, false)
                .unwrap()
                .iter()
                .all(|finding| finding.severity != diagnostics::Severity::Error)
        );
        let backup = backup_fixed(&paths, BackupReason::Manual).unwrap();
        let backup_root = fs::canonicalize(&paths.backups).unwrap();
        assert!(backup.database.starts_with(&backup_root));
        assert_eq!(paths.database.file_name().unwrap(), "mnab.sqlite3");
        let mut catalog = BudgetCatalog::default();
        catalog.refresh(&paths).unwrap();
        let id = catalog.entries()[0].budget_id;
        let report = catalog
            .repair(&paths, id, crate::storage::repair::RepairRequest::Reindex)
            .unwrap();
        let fixed_database = fs::canonicalize(&paths.database).unwrap();
        assert_eq!(report.replacement, fixed_database);
    }
}
