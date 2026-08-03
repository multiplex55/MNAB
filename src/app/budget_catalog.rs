//! Discovery and lifecycle management for portable, managed budget databases.
//!
//! Paths in this module are capabilities: a path is not returned as managed until
//! its canonical target has been proven to be a regular file below `budgets`.

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

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArchiveState {
    #[default]
    Active,
    Archived,
}

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
    pub archive_state: ArchiveState,
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
    #[error("exact budget name confirmation did not match")]
    ConfirmationMismatch,
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
    pub fn confirm_name(&self, id: BudgetId, confirmation: &str) -> Result<(), CatalogError> {
        let entry = self
            .entries
            .iter()
            .find(|e| e.budget_id == id)
            .ok_or(CatalogError::NotFound)?;
        if confirmation == entry.display_name {
            Ok(())
        } else {
            Err(CatalogError::ConfirmationMismatch)
        }
    }

    /// Refreshes filesystem facts without discarding missing recent entries.
    pub fn refresh(&mut self, paths: &PortablePaths) -> Result<(), CatalogError> {
        let root = fs::canonicalize(&paths.budgets)?;
        for entry in &mut self.entries {
            entry.file_existence = if entry.database_path.is_file() {
                FileExistence::Present
            } else {
                FileExistence::Missing
            };
        }
        for item in fs::read_dir(&root)? {
            let item = item?;
            let lexical = item.path();
            let Ok(path) = managed_file(&root, &lexical) else {
                continue;
            };
            if self.entries.iter().any(|e| e.database_path == path) {
                continue;
            }
            let Some(info) = inspect(&path)? else {
                continue;
            };
            self.entries.push(BudgetCatalogEntry {
                budget_id: info.id,
                display_name: info.name,
                database_path: path,
                schema_version: info.version,
                archive_state: ArchiveState::Active,
                last_successful_open: None,
                last_validation: None,
                file_existence: FileExistence::Present,
                recognition: DatabaseRecognition::Mnab,
            });
        }
        Ok(())
    }

    /// Most recently opened active budgets first; unavailable rows remain visible.
    #[must_use]
    pub fn recent(&self) -> Vec<&BudgetCatalogEntry> {
        let mut result: Vec<_> = self
            .entries
            .iter()
            .filter(|e| e.archive_state == ArchiveState::Active)
            .collect();
        result.sort_by(|a, b| {
            b.last_successful_open
                .cmp(&a.last_successful_open)
                .then_with(|| a.display_name.cmp(&b.display_name))
        });
        result
    }

    pub fn set_archived(&mut self, id: BudgetId, archived: bool) -> Result<(), CatalogError> {
        self.entry_mut(id)?.archive_state = if archived {
            ArchiveState::Archived
        } else {
            ArchiveState::Active
        };
        Ok(())
    }
    pub fn remove_from_recents(&mut self, id: BudgetId) -> bool {
        let before = self.entries.len();
        self.entries.retain(|e| e.budget_id != id);
        before != self.entries.len()
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
        let root = fs::canonicalize(&paths.budgets)?;
        let path = managed_file(&root, &entry.database_path)?;
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
    pub fn prepare_open(
        &self,
        paths: &PortablePaths,
        selected: &Path,
        repaint: impl Fn() + Send + 'static,
    ) -> Result<PreparedBudget, CatalogError> {
        let root = fs::canonicalize(&paths.budgets)?;
        let path = managed_file(&root, selected)?; // 1: managed path
        let info = inspect(&path)?
            .ok_or_else(|| CatalogError::NotMnab("header or schema not recognized".into()))?; // 2
        let validation = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY)?; // 3
        if info.version > LATEST_SCHEMA_VERSION {
            return Err(CatalogError::FutureSchema {
                found: info.version,
                supported: LATEST_SCHEMA_VERSION,
            });
        } // 4
        let findings = diagnostics::all(&validation, false)?; // 5
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

    pub fn delete(
        &mut self,
        paths: &PortablePaths,
        id: BudgetId,
        confirmation: &str,
    ) -> Result<DeletionResult, CatalogError> {
        self.confirm_name(id, confirmation)?;
        let entry = self
            .entries
            .iter()
            .find(|e| e.budget_id == id)
            .expect("confirmed entry");
        let root = fs::canonicalize(&paths.budgets)?;
        let path = managed_file(&root, &entry.database_path)?;
        let targets = [
            path.clone(),
            PathBuf::from(format!("{}-wal", path.display())),
            PathBuf::from(format!("{}-shm", path.display())),
        ];
        let mut result = DeletionResult::default();
        for target in targets {
            match fs::remove_file(&target) {
                Ok(()) => result.removed.push(target),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => result.failed.push(RemovalFailure {
                    path: target,
                    error: e.to_string(),
                }),
            }
        }
        if result.failed.is_empty() {
            self.remove_from_recents(id);
        }
        Ok(result)
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
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DeletionResult {
    pub removed: Vec<PathBuf>,
    pub failed: Vec<RemovalFailure>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemovalFailure {
    pub path: PathBuf,
    pub error: String,
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
    let mut storage = PortableBudgetStorage::new(&paths.budgets);
    budget_service::create_budget(&mut storage, request)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths() -> (tempfile::TempDir, PortablePaths) {
        let dir = tempfile::tempdir().unwrap();
        let paths = PortablePaths::from_executable(&dir.path().join("mnab.exe")).unwrap();
        (dir, paths)
    }
    fn request(name: &str, file: &str) -> CreateBudget {
        CreateBudget {
            name: name.into(),
            database_filename: file.into(),
            initial_month: BudgetMonth::new(2026, 8).unwrap(),
            currency: "USD".into(),
            starter_content: true,
        }
    }

    #[test]
    fn creation_discovery_rename_archive_remove_and_missing_are_independent() {
        let (_dir, paths) = paths();
        let budget = create_managed(&paths, request("Household", "independent.sqlite3")).unwrap();
        let mut catalog = BudgetCatalog::default();
        catalog.refresh(&paths).unwrap();
        assert_eq!(catalog.entries()[0].budget_id, budget.id);
        let original = catalog.entries()[0].database_path.clone();
        catalog
            .rename(&paths, budget.id, "Renamed display")
            .unwrap();
        assert_eq!(catalog.entries()[0].database_path, original);
        catalog.set_archived(budget.id, true).unwrap();
        assert!(catalog.recent().is_empty());
        catalog.set_archived(budget.id, false).unwrap();
        fs::remove_file(original).unwrap();
        catalog.refresh(&paths).unwrap();
        assert_eq!(catalog.entries()[0].file_existence, FileExistence::Missing);
        assert!(catalog.remove_from_recents(budget.id));
    }

    #[test]
    fn external_copy_is_collision_safe_and_does_not_modify_source() {
        let (_external_dir, external_paths) = paths();
        create_managed(&external_paths, request("Source", "source.sqlite3")).unwrap();
        let source = external_paths.budgets.join("source.sqlite3");
        let before = fs::read(&source).unwrap();
        let (_managed_dir, managed) = paths();
        let catalog = BudgetCatalog::default();
        let first = catalog.import_external(&managed, &source).unwrap();
        let second = catalog.import_external(&managed, &source).unwrap();
        assert_ne!(first, second);
        assert_eq!(fs::read(source).unwrap(), before);
    }

    #[test]
    fn rejects_traversal_non_files_and_symlink_escape() {
        let (dir, paths) = paths();
        let root = fs::canonicalize(&paths.budgets).unwrap();
        assert!(managed_file(&root, &paths.budgets.join("../settings.json")).is_err());
        assert!(managed_file(&root, &paths.budgets).is_err());
        #[cfg(unix)]
        {
            let outside = dir.path().join("outside");
            fs::write(&outside, b"x").unwrap();
            std::os::unix::fs::symlink(outside, paths.budgets.join("escape")).unwrap();
            assert!(managed_file(&root, &paths.budgets.join("escape")).is_err());
        }
    }

    #[test]
    fn recent_order_and_exact_confirmation_and_sidecars() {
        let (_dir, paths) = paths();
        let old = create_managed(&paths, request("Old", "old.sqlite3")).unwrap();
        let new = create_managed(&paths, request("New", "new.sqlite3")).unwrap();
        let mut catalog = BudgetCatalog::default();
        catalog.refresh(&paths).unwrap();
        catalog.entry_mut(old.id).unwrap().last_successful_open = Some(OffsetDateTime::UNIX_EPOCH);
        catalog.entry_mut(new.id).unwrap().last_successful_open = Some(OffsetDateTime::now_utc());
        assert_eq!(catalog.recent()[0].budget_id, new.id);
        assert!(matches!(
            catalog.delete(&paths, new.id, "new"),
            Err(CatalogError::ConfirmationMismatch)
        ));
        let path = catalog
            .entries()
            .iter()
            .find(|e| e.budget_id == new.id)
            .unwrap()
            .database_path
            .clone();
        fs::write(format!("{}-wal", path.display()), b"wal").unwrap();
        fs::write(format!("{}-shm", path.display()), b"shm").unwrap();
        let result = catalog.delete(&paths, new.id, "New").unwrap();
        assert_eq!(result.removed.len(), 3);
        assert!(result.failed.is_empty());
    }
}
