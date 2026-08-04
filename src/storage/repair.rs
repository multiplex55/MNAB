//! Explicit, bounded database repairs. Financial rows are never interpreted or changed.

use rusqlite::{Connection, OpenFlags};
use std::{
    fs,
    path::{Path, PathBuf},
};
use uuid::Uuid;

use super::{
    diagnostics::{self, Finding, Severity},
    migration,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RepairRequest {
    WalRecoveryAndCheckpoint,
    Reindex,
    Reconstruct,
    CompleteMigrations,
}

#[derive(Debug)]
pub struct RepairReport {
    pub replacement: PathBuf,
    pub findings: Vec<Finding>,
    pub backup: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedRepairPlan {
    pub selected_budget_path: PathBuf,
    pub request: RepairRequest,
    pub preview_findings: Vec<Finding>,
    pub proposed_actions: Vec<&'static str>,
}

impl ManagedRepairPlan {
    pub fn for_explicit_selection(
        path: &Path,
        request: RepairRequest,
    ) -> Result<Self, RepairError> {
        if !path.is_file() {
            return Err(RepairError::ExplicitSelectionRequired);
        }
        let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let preview_findings = diagnostics::all(&connection, true)?;
        Ok(Self {
            selected_budget_path: path.to_owned(),
            request,
            preview_findings,
            proposed_actions: match request {
                RepairRequest::WalRecoveryAndCheckpoint => {
                    vec!["Recover WAL contents", "Checkpoint the database"]
                }
                RepairRequest::Reindex => vec!["Rebuild SQLite indexes"],
                RepairRequest::Reconstruct => {
                    vec!["Copy reachable SQLite pages into a replacement database"]
                }
                RepairRequest::CompleteMigrations => vec!["Run missing immutable migrations"],
            },
        })
    }
}

pub fn repair_managed(
    plan: &ManagedRepairPlan,
    confirmed_path: &Path,
) -> Result<RepairReport, RepairError> {
    if plan.selected_budget_path != confirmed_path {
        return Err(RepairError::ExplicitSelectionRequired);
    }
    repair(&plan.selected_budget_path, plan.request)
}

#[derive(Debug, thiserror::Error)]
pub enum RepairError {
    #[error("filesystem preservation/replacement failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("SQLite repair failed: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("migration repair failed: {0}")]
    Migration(#[from] migration::MigrationError),
    #[error("repaired copy failed complete diagnostics")]
    Validation { findings: Vec<Finding> },
    #[error("repair requires an explicitly selected managed budget")]
    ExplicitSelectionRequired,
    #[error("mandatory repair backup failed validation")]
    BackupValidation,
}

/// Repairs a private copy, validates it completely, then uses an original-preserving
/// rename sequence. On any validation failure the original path is untouched.
pub fn repair(path: &Path, request: RepairRequest) -> Result<RepairReport, RepairError> {
    let parent = path
        .parent()
        .ok_or_else(|| std::io::Error::other("database has no parent"))?;
    let candidate = parent.join(format!(".repair-{}.sqlite3", Uuid::new_v4()));
    let preserved = parent.join(format!(
        "{}.pre-repair-{}",
        path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("budget.sqlite3"),
        Uuid::new_v4()
    ));
    let result = (|| {
        let backup = verified_safety_backup(path)?;
        reconstruct_copy(path, &candidate, request)?;
        let validation = Connection::open_with_flags(&candidate, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let findings = diagnostics::all(&validation, true)?;
        drop(validation);
        if findings.iter().any(|f| f.severity == Severity::Error) {
            return Err(RepairError::Validation { findings });
        }
        fs::rename(path, &preserved).map_err(|e| {
            std::io::Error::new(
                e.kind(),
                format!(
                    "could not preserve {} as {}: {e}",
                    path.display(),
                    preserved.display()
                ),
            )
        })?;
        if let Err(error) = fs::rename(&candidate, path) {
            if let Err(rollback) = fs::rename(&preserved, path) {
                return Err(RepairError::Io(std::io::Error::other(format!(
                    "replacement failed: {error}; rollback failed: {rollback}; preserved copy is {}",
                    preserved.display()
                ))));
            }
            return Err(RepairError::Io(std::io::Error::new(
                error.kind(),
                format!("replacement failed and original was restored: {error}"),
            )));
        }
        sync_dir(parent)?;
        Ok(RepairReport {
            replacement: path.to_owned(),
            findings,
            backup,
        })
    })();
    if result.is_err() {
        let _ = fs::remove_file(&candidate);
    }
    result
}

fn verified_safety_backup(path: &Path) -> Result<PathBuf, RepairError> {
    let parent = path
        .parent()
        .ok_or_else(|| std::io::Error::other("database has no parent"))?;
    let backup = parent.join(format!(
        "{}.mandatory-pre-repair-{}",
        path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("budget.sqlite3"),
        Uuid::new_v4()
    ));
    fs::copy(path, &backup)?;
    let validation = Connection::open_with_flags(&backup, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    if diagnostics::all(&validation, true)?
        .iter()
        .any(|finding| finding.severity == Severity::Error)
    {
        return Err(RepairError::BackupValidation);
    }
    Ok(backup)
}

fn reconstruct_copy(
    source: &Path,
    target: &Path,
    request: RepairRequest,
) -> Result<(), RepairError> {
    let source = Connection::open(source)?;
    if request == RepairRequest::WalRecoveryAndCheckpoint {
        source.execute_batch("PRAGMA wal_checkpoint(FULL)")?;
    }
    let mut destination = Connection::open(target)?;
    rusqlite::backup::Backup::new(&source, &mut destination)?.run_to_completion(
        64,
        std::time::Duration::from_millis(5),
        None,
    )?;
    match request {
        RepairRequest::Reindex => destination.execute_batch("REINDEX")?,
        RepairRequest::CompleteMigrations => migration::migrate(&mut destination, target)?,
        RepairRequest::WalRecoveryAndCheckpoint | RepairRequest::Reconstruct => {}
    }
    destination.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")?;
    Ok(())
}

#[cfg(unix)]
fn sync_dir(path: &Path) -> std::io::Result<()> {
    fs::File::open(path)?.sync_all()
}
#[cfg(not(unix))]
fn sync_dir(_path: &Path) -> std::io::Result<()> {
    // `File::open` on a directory fails with ERROR_ACCESS_DENIED on Windows.
    // Both database files were closed before the rename sequence, which is the
    // durability boundary available through the portable standard library.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failed_validation_leaves_original_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.sqlite3");
        let c = Connection::open(&path).unwrap();
        c.execute_batch("CREATE TABLE budgets(id TEXT); INSERT INTO budgets VALUES('x');")
            .unwrap();
        drop(c);
        let before = fs::read(&path).unwrap();
        assert!(repair(&path, RepairRequest::Reconstruct).is_err());
        assert_eq!(fs::read(path).unwrap(), before);
    }

    #[test]
    fn successful_repair_preserves_original_copy() {
        let dir = tempfile::tempdir().unwrap();
        let paths = crate::app::portable_paths::PortablePaths::from_executable(
            &dir.path().join("mnab.exe"),
        )
        .unwrap();
        crate::app::budget_catalog::create_managed(
            &paths,
            crate::service::budget_service::CreateBudget {
                name: "Repair me".into(),
                database_filename: "repair.sqlite3".into(),
                initial_month: crate::domain::BudgetMonth::new(2026, 8).unwrap(),
                currency: "USD".into(),
                starter_content: true,
            },
        )
        .unwrap();
        let path = paths.budgets.join("repair.sqlite3");
        repair(&path, RepairRequest::Reindex).unwrap();
        assert!(path.is_file());
        assert!(
            std::fs::read_dir(&paths.budgets)
                .unwrap()
                .flatten()
                .any(|e| e.file_name().to_string_lossy().contains("pre-repair"))
        );
        let c = Connection::open(path).unwrap();
        assert!(diagnostics::all(&c, true).unwrap().is_empty());
    }
}
