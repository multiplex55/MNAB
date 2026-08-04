use super::{connection::open_primary, migration::MigrationError};
use rusqlite::Connection;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

pub type RequestId = u64;
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Generation {
    pub budget: u64,
    pub view: u64,
}
/// Typed worker protocol.  Keeping each use-case in its own operation type prevents the UI from
/// smuggling closures, connections, or arbitrary SQL across the thread boundary.
#[derive(Debug)]
pub enum WorkerOperation {
    Session(SessionOperation),
    Financial(FinancialOperation),
    View(ViewOperation),
    Register(RegisterPageOperation),
    Search(GlobalSearchOperation),
    Import(ImportOperation),
    Diagnostics(DiagnosticsOperation),
    Report(ReportOperation),
    Occurrences(OccurrenceOperation),
}
#[derive(Debug)]
pub enum SessionOperation {
    Open,
    Close,
    Health,
}
#[derive(Debug)]
pub enum FinancialOperation {
    Command(crate::app::command::CommandEnvelope),
}
#[derive(Debug)]
pub enum ViewOperation {
    BudgetCount,
}
#[derive(Debug)]
pub struct RegisterPageOperation {
    pub offset: u32,
    pub limit: u32,
}
#[derive(Debug)]
pub struct GlobalSearchOperation {
    pub text: String,
    pub limit: u32,
}
#[derive(Debug)]
pub enum ImportOperation {
    Parse {
        path: PathBuf,
    },
    ParseCsv {
        path: PathBuf,
        preset: Box<crate::importing::csv_mapping::CsvMappingPreset>,
    },
    Stage,
    Apply,
}
#[derive(Debug)]
pub enum DiagnosticsOperation {
    Run,
}
#[derive(Debug)]
pub struct ReportOperation {
    pub budget_id: crate::domain::BudgetId,
    pub request: crate::domain::ReportRequest,
}
#[derive(Debug)]
pub struct OccurrenceOperation {
    pub through: time::Date,
}
#[derive(Debug)]
pub struct StorageRequest {
    pub id: RequestId,
    pub generation: Generation,
    pub operation: WorkerOperation,
}
#[derive(Debug, PartialEq)]
pub enum TypedResult {
    Healthy,
    Count(i64),
    /// A transaction committed successfully. Only this result is eligible for undo history.
    Mutation {
        command: crate::app::command::FinancialCommand,
        inverse: crate::app::command::FinancialCommand,
    },
    Report(crate::domain::ReportResult),
    RegisterPage,
    SearchResults,
    ImportParsed,
    ImportStatement(Box<crate::importing::ImportedStatement>),
    ImportStaged,
    ImportApplied,
    Diagnostics,
    OccurrencesGenerated,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SafeUserError {
    pub message: &'static str,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiagnosticContext {
    pub operation: &'static str,
    pub detail: String,
}
#[derive(Debug)]
pub enum StorageResponse {
    Completed {
        id: RequestId,
        command_id: Option<crate::app::command::CommandId>,
        correlation_id: Option<crate::app::command::CorrelationId>,
        generation: Generation,
        result: Result<TypedResult, WorkerError>,
        invalidations: Option<crate::app::view_invalidation::ViewInvalidations>,
        user_error: Option<SafeUserError>,
        /// This context is for structured logging only and must never be displayed to users.
        diagnostic: Option<DiagnosticContext>,
    },
    Progress {
        id: RequestId,
        command_id: Option<crate::app::command::CommandId>,
        correlation_id: Option<crate::app::command::CorrelationId>,
        generation: Generation,
        completed: u64,
        total: Option<u64>,
    },
    Terminated,
}
#[derive(Debug, thiserror::Error)]
pub enum WorkerError {
    #[error("query was cancelled")]
    Cancelled,
    #[error("worker is shutting down")]
    Shutdown,
    #[error("repository error: {0}")]
    Repository(String),
    #[error("storage worker terminated")]
    Terminated,
}
enum Message {
    Work(StorageRequest),
    Shutdown,
}

pub struct StorageWorker {
    sender: mpsc::Sender<Message>,
    responses: mpsc::Receiver<StorageResponse>,
    stopping: Arc<AtomicBool>,
    handle: Option<JoinHandle<Result<(), WorkerError>>>,
}
impl StorageWorker {
    pub fn start(path: &Path, repaint: impl Fn() + Send + 'static) -> Result<Self, MigrationError> {
        let (tx, rx) = mpsc::channel();
        let (response_tx, responses) = mpsc::channel();
        let (startup_tx, startup_rx) = mpsc::sync_channel(1);
        let stopping = Arc::new(AtomicBool::new(false));
        let thread_stopping = stopping.clone();
        let path = path.to_owned();
        let handle = thread::Builder::new()
            .name("mnab-storage".into())
            .spawn(move || run(path, rx, response_tx, startup_tx, thread_stopping, repaint))
            .map_err(MigrationError::Backup)?;
        startup_rx.recv().map_err(|_| {
            MigrationError::Backup(std::io::Error::other(
                "storage worker terminated during startup",
            ))
        })??;
        Ok(Self {
            sender: tx,
            responses,
            stopping,
            handle: Some(handle),
        })
    }
    pub fn submit(&self, request: StorageRequest) -> Result<(), WorkerError> {
        if self.stopping.load(Ordering::Acquire) {
            return Err(WorkerError::Shutdown);
        }
        self.sender
            .send(Message::Work(request))
            .map_err(|_| WorkerError::Terminated)
    }
    pub fn try_response(&self) -> Option<StorageResponse> {
        self.responses.try_recv().ok()
    }
    /// Drains the complete ready queue. Frame loops should use this rather than imposing one-frame
    /// latency per response.
    pub fn drain_ready(&self) -> Vec<StorageResponse> {
        self.responses.try_iter().collect()
    }
    pub fn response_timeout(&self, timeout: Duration) -> Option<StorageResponse> {
        self.responses.recv_timeout(timeout).ok()
    }
    pub fn shutdown(&mut self) -> Result<(), WorkerError> {
        if !self.stopping.swap(true, Ordering::AcqRel) {
            let _ = self.sender.send(Message::Shutdown);
        }
        if let Some(h) = self.handle.take() {
            h.join().map_err(|_| WorkerError::Terminated)??;
        }
        Ok(())
    }
}
impl Drop for StorageWorker {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

#[allow(clippy::needless_pass_by_value)] // Values are moved to, and owned by, this thread.
fn run(
    path: PathBuf,
    rx: mpsc::Receiver<Message>,
    tx: mpsc::Sender<StorageResponse>,
    startup: mpsc::SyncSender<Result<(), MigrationError>>,
    stopping: Arc<AtomicBool>,
    repaint: impl Fn(),
) -> Result<(), WorkerError> {
    let connection = match open_primary(&path) {
        Ok(connection) => {
            let _ = startup.send(Ok(()));
            connection
        }
        Err(error) => {
            let _ = startup.send(Err(error));
            return Ok(());
        }
    };
    let mut report_cache = ReportCache::default();
    while let Ok(message) = rx.recv() {
        match message {
            Message::Shutdown => break,
            Message::Work(request) => {
                let association = match &request.operation {
                    WorkerOperation::Financial(FinancialOperation::Command(e)) => {
                        (Some(e.command_id), Some(e.correlation_id))
                    }
                    _ => (None, None),
                };
                if matches!(
                    request.operation,
                    WorkerOperation::Import(_)
                        | WorkerOperation::Diagnostics(_)
                        | WorkerOperation::Report(_)
                        | WorkerOperation::Occurrences(_)
                ) {
                    let _ = tx.send(StorageResponse::Progress {
                        id: request.id,
                        command_id: association.0,
                        correlation_id: association.1,
                        generation: request.generation,
                        completed: 0,
                        total: None,
                    });
                    repaint();
                }
                let result = if stopping.load(Ordering::Acquire) {
                    Err(WorkerError::Cancelled)
                } else {
                    execute(&connection, &request.operation, &mut report_cache)
                };
                repaint();
                let _ = tx.send(StorageResponse::Completed {
                    id: request.id,
                    command_id: association.0,
                    correlation_id: association.1,
                    generation: request.generation,
                    result,
                    invalidations: None,
                    user_error: None,
                    diagnostic: None,
                });
            }
        }
    }
    while let Ok(Message::Work(r)) = rx.try_recv() {
        repaint();
        let _ = tx.send(StorageResponse::Completed {
            id: r.id,
            command_id: match &r.operation {
                WorkerOperation::Financial(FinancialOperation::Command(e)) => Some(e.command_id),
                _ => None,
            },
            correlation_id: match &r.operation {
                WorkerOperation::Financial(FinancialOperation::Command(e)) => {
                    Some(e.correlation_id)
                }
                _ => None,
            },
            generation: r.generation,
            result: Err(WorkerError::Cancelled),
            invalidations: None,
            user_error: Some(SafeUserError {
                message: "The operation was cancelled.",
            }),
            diagnostic: None,
        });
    }
    connection
        .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
        .map_err(|e| WorkerError::Repository(format!("WAL checkpoint failed: {e}")))?;
    drop(connection);
    let _ = tx.send(StorageResponse::Terminated);
    repaint();
    Ok(())
}
#[derive(Default)]
struct ReportCache {
    entries: HashMap<(crate::domain::BudgetId, String, u64), crate::domain::ReportResult>,
}
fn execute(
    c: &Connection,
    op: &WorkerOperation,
    cache: &mut ReportCache,
) -> Result<TypedResult, WorkerError> {
    match op {
        WorkerOperation::Session(
            SessionOperation::Health | SessionOperation::Open | SessionOperation::Close,
        ) => Ok(TypedResult::Healthy),
        WorkerOperation::View(ViewOperation::BudgetCount) => c
            .query_row("SELECT count(*) FROM budgets", [], |r| r.get(0))
            .map(TypedResult::Count)
            .map_err(|e| WorkerError::Repository(e.to_string())),
        WorkerOperation::Report(ReportOperation { budget_id, request }) => {
            let store = crate::storage::query_store::QueryStore::new(c);
            let revision = store
                .report_revision(*budget_id, request.kind)
                .map_err(|e| WorkerError::Repository(e.to_string()))?;
            let normalized = serde_json::to_string(request)
                .map_err(|e| WorkerError::Repository(e.to_string()))?;
            let key = (*budget_id, normalized, revision);
            if let Some(result) = cache.entries.get(&key) {
                return Ok(TypedResult::Report(result.clone()));
            }
            let result = store
                .report(*budget_id, request)
                .map_err(|e| WorkerError::Repository(e.to_string()))?;
            cache.entries.retain(|(budget, _, old_revision), _| {
                budget != budget_id || *old_revision == revision
            });
            cache.entries.insert(key, result.clone());
            Ok(TypedResult::Report(result))
        }
        WorkerOperation::Register(_) => Ok(TypedResult::RegisterPage),
        WorkerOperation::Search(search) => {
            // Parsing and planning live beside the worker-side database execution;
            // the UI thread only schedules debounced text and renders typed results.
            let ast = crate::app::search::parse(&search.text)
                .map_err(|_| WorkerError::Repository("invalid search expression".into()))?;
            let plan = crate::app::search::compile(&ast);
            let bounded_limit = search.limit.clamp(1, 100);
            let sql = format!(
                "SELECT transactions.id FROM transactions \
                 JOIN accounts ON accounts.id=transactions.account_id \
                 LEFT JOIN payees ON payees.id=transactions.payee_id \
                 LEFT JOIN categories ON categories.id=transactions.category_id \
                 {} LIMIT ?",
                if plan.where_sql.is_empty() {
                    String::new()
                } else {
                    format!("WHERE {}", plan.where_sql)
                }
            );
            let mut values = plan
                .binds
                .into_iter()
                .map(|value| match value {
                    crate::app::search::BindValue::Text(value) => {
                        rusqlite::types::Value::Text(value)
                    }
                    crate::app::search::BindValue::Integer(value) => {
                        rusqlite::types::Value::Integer(value)
                    }
                })
                .collect::<Vec<_>>();
            values.push(rusqlite::types::Value::Integer(i64::from(bounded_limit)));
            let mut statement = c
                .prepare(&sql)
                .map_err(|error| WorkerError::Repository(error.to_string()))?;
            let mut rows = statement
                .query(rusqlite::params_from_iter(values))
                .map_err(|error| WorkerError::Repository(error.to_string()))?;
            while rows
                .next()
                .map_err(|error| WorkerError::Repository(error.to_string()))?
                .is_some()
            {}
            Ok(TypedResult::SearchResults)
        }
        WorkerOperation::Import(ImportOperation::Parse { path }) => {
            let bytes = std::fs::read(path).map_err(|error| {
                WorkerError::Repository(format!("statement read failed: {error}"))
            })?;
            match crate::importing::source::detect(&bytes, Some(path)) {
                crate::importing::source::Detection::Certain(
                    crate::importing::source::ImportFormat::Ofx,
                ) => crate::importing::ofx::parse(&bytes)
                    .map(Box::new)
                    .map(TypedResult::ImportStatement)
                    .map_err(|error| WorkerError::Repository(error.to_string())),
                _ => Ok(TypedResult::ImportParsed),
            }
        }
        WorkerOperation::Import(ImportOperation::ParseCsv { path, preset }) => {
            let bytes = std::fs::read(path).map_err(|error| {
                WorkerError::Repository(format!("statement read failed: {error}"))
            })?;
            crate::importing::csv::parse_preset(&bytes, preset)
                .map(Box::new)
                .map(TypedResult::ImportStatement)
                .map_err(|error| WorkerError::Repository(error.to_string()))
        }
        WorkerOperation::Import(ImportOperation::Stage) => Ok(TypedResult::ImportStaged),
        WorkerOperation::Import(ImportOperation::Apply) => Ok(TypedResult::ImportApplied),
        WorkerOperation::Diagnostics(_) => Ok(TypedResult::Diagnostics),
        WorkerOperation::Occurrences(_) => Ok(TypedResult::OccurrencesGenerated),
        WorkerOperation::Financial(FinancialOperation::Command(envelope)) => {
            let crate::app::command::ApplicationAction::Financial(command) = &envelope.payload
            else {
                return Err(WorkerError::Repository(
                    "non-financial command in financial request".into(),
                ));
            };
            Ok(TypedResult::Mutation {
                command: command.clone(),
                inverse: command.clone(),
            })
        }
    }
}

/// Returns true only for the latest request and currently open budget/view generation.
pub fn response_is_current(
    response: &StorageResponse,
    expected_id: RequestId,
    generation: Generation,
) -> bool {
    matches!(response,StorageResponse::Completed{id,generation:g,..} | StorageResponse::Progress{id,generation:g,..} if *id==expected_id && *g==generation)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn correlates_repaints_filters_and_shuts_down() {
        let dir = tempfile::tempdir().unwrap();
        let paints = Arc::new(AtomicUsize::new(0));
        let callback_count = paints.clone();
        let mut worker = StorageWorker::start(&dir.path().join("db.sqlite3"), move || {
            callback_count.fetch_add(1, Ordering::Relaxed);
        })
        .unwrap();
        let generation = Generation { budget: 4, view: 9 };
        worker
            .submit(StorageRequest {
                id: 7,
                generation,
                operation: WorkerOperation::Session(SessionOperation::Health),
            })
            .unwrap();
        let response = worker.response_timeout(Duration::from_secs(2)).unwrap();
        assert!(response_is_current(&response, 7, generation));
        assert!(!response_is_current(&response, 8, generation));
        assert!(paints.load(Ordering::Relaxed) > 0);
        worker.shutdown().unwrap();
        assert!(matches!(
            worker.submit(StorageRequest {
                id: 8,
                generation,
                operation: WorkerOperation::Session(SessionOperation::Health)
            }),
            Err(WorkerError::Shutdown)
        ));
    }
}
