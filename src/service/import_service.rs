//! Atomic import coordination. Parsing/staging is intentionally separate from
//! persistence; raw bytes remain exclusively in this staging object/archive.
use crate::importing::{
    ImportedStatement,
    preview::{ImportPreview, ReviewDecision},
    source::ImportError,
};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
};
use thiserror::Error;

#[derive(Clone, Debug)]
pub struct StagedImport {
    pub source_name: String,
    pub raw_source: Vec<u8>,
    pub statement: ImportedStatement,
    pub preview: ImportPreview,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BatchState {
    Applied,
    ArchiveRetryRequired {
        intended_path: PathBuf,
        checksum: String,
    },
}
#[derive(Clone, Debug)]
pub struct CommittedBatch {
    pub id: String,
    pub state: BatchState,
}

pub trait ImportUnitOfWork {
    type Transaction;
    fn pre_operation_backup(&mut self) -> Result<(), String>;
    fn begin(&mut self) -> Result<Self::Transaction, String>;
    fn insert_batch(
        transaction: &mut Self::Transaction,
        source_name: &str,
    ) -> Result<String, String>;
    /// Implementations insert accepted rows unapproved, their identifiers and
    /// fingerprints, decisions, and payee usage through the same transaction.
    fn insert_candidate(
        transaction: &mut Self::Transaction,
        candidate: &crate::importing::preview::ImportProposal,
    ) -> Result<(), String>;
    fn commit(transaction: Self::Transaction) -> Result<(), String>;
    fn mark_archive_retry(
        &mut self,
        batch_id: &str,
        path: &Path,
        checksum: &str,
    ) -> Result<(), String>;
}

#[derive(Debug, Error)]
pub enum ImportServiceError {
    #[error("parse failed: {0}")]
    Parse(#[from] ImportError),
    #[error("import database operation failed: {0}")]
    Database(String),
    #[error("source archive failed after database commit; retry is required: {0}")]
    Archive(String),
}

pub struct ImportService<R> {
    repository: R,
    archive_directory: PathBuf,
}
impl<R: ImportUnitOfWork> ImportService<R> {
    #[must_use]
    pub fn new(repository: R, archive_directory: PathBuf) -> Self {
        Self {
            repository,
            archive_directory,
        }
    }
    pub fn commit(&mut self, staged: &StagedImport) -> Result<CommittedBatch, ImportServiceError> {
        self.repository
            .pre_operation_backup()
            .map_err(ImportServiceError::Database)?;
        let mut transaction = self
            .repository
            .begin()
            .map_err(ImportServiceError::Database)?;
        let id = R::insert_batch(&mut transaction, &staged.source_name)
            .map_err(ImportServiceError::Database)?;
        for candidate in &staged.preview.candidates {
            if candidate.decision == ReviewDecision::Accept {
                R::insert_candidate(&mut transaction, candidate)
                    .map_err(ImportServiceError::Database)?;
            }
        }
        R::commit(transaction).map_err(ImportServiceError::Database)?;
        let checksum = format!("{:x}", Sha256::digest(&staged.raw_source));
        let safe_name = Path::new(&staged.source_name)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("statement")
            .replace(['/', '\\'], "_");
        let path = self
            .archive_directory
            .join(format!("{id}-{checksum}-{safe_name}"));
        if archive(&path, &staged.raw_source).is_err() {
            self.repository
                .mark_archive_retry(&id, &path, &checksum)
                .map_err(ImportServiceError::Database)?;
            return Ok(CommittedBatch {
                id,
                state: BatchState::ArchiveRetryRequired {
                    intended_path: path,
                    checksum,
                },
            });
        }
        Ok(CommittedBatch {
            id,
            state: BatchState::Applied,
        })
    }
}
fn archive(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    fs::create_dir_all(
        path.parent()
            .ok_or_else(|| std::io::Error::other("archive path has no parent"))?,
    )?;
    let temporary = path.with_extension("partial");
    fs::write(&temporary, bytes)?;
    fs::rename(temporary, path)
}
