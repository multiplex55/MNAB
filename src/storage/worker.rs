use super::{connection::open_primary, migration::MigrationError};
use rusqlite::Connection;
use std::{
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
#[derive(Debug)]
pub enum StorageOperation {
    Health,
    BudgetCount,
    Delay(Duration),
    /// An owned immutable snapshot keeps all repository access and calculation off the UI thread.
    Report {
        request: crate::domain::ReportRequest,
        data: Box<crate::domain::OwnedReportData>,
    },
}
#[derive(Debug)]
pub struct StorageRequest {
    pub id: RequestId,
    pub generation: Generation,
    pub operation: StorageOperation,
}
#[derive(Debug, PartialEq)]
pub enum StorageResult {
    Healthy,
    Count(i64),
    Completed,
    Report(crate::domain::ReportResult),
}
#[derive(Debug)]
pub enum StorageResponse {
    Completed {
        id: RequestId,
        generation: Generation,
        result: Result<StorageResult, WorkerError>,
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
    handle: Option<JoinHandle<()>>,
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
    pub fn response_timeout(&self, timeout: Duration) -> Option<StorageResponse> {
        self.responses.recv_timeout(timeout).ok()
    }
    pub fn shutdown(&mut self) -> Result<(), WorkerError> {
        if !self.stopping.swap(true, Ordering::AcqRel) {
            let _ = self.sender.send(Message::Shutdown);
        }
        if let Some(h) = self.handle.take() {
            h.join().map_err(|_| WorkerError::Terminated)?;
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
) {
    let connection = match open_primary(&path) {
        Ok(connection) => {
            let _ = startup.send(Ok(()));
            connection
        }
        Err(error) => {
            let _ = startup.send(Err(error));
            return;
        }
    };
    while let Ok(message) = rx.recv() {
        match message {
            Message::Shutdown => break,
            Message::Work(request) => {
                let result = if stopping.load(Ordering::Acquire) {
                    Err(WorkerError::Cancelled)
                } else {
                    execute(&connection, &request.operation)
                };
                let _ = tx.send(StorageResponse::Completed {
                    id: request.id,
                    generation: request.generation,
                    result,
                });
                repaint();
            }
        }
    }
    while let Ok(Message::Work(r)) = rx.try_recv() {
        let _ = tx.send(StorageResponse::Completed {
            id: r.id,
            generation: r.generation,
            result: Err(WorkerError::Cancelled),
        });
        repaint();
    }
    let _ = connection.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
    drop(connection);
    let _ = tx.send(StorageResponse::Terminated);
    repaint();
}
fn execute(c: &Connection, op: &StorageOperation) -> Result<StorageResult, WorkerError> {
    match op {
        StorageOperation::Health => Ok(StorageResult::Healthy),
        StorageOperation::BudgetCount => c
            .query_row("SELECT count(*) FROM budgets", [], |r| r.get(0))
            .map(StorageResult::Count)
            .map_err(|e| WorkerError::Repository(e.to_string())),
        StorageOperation::Delay(d) => {
            thread::sleep(*d);
            Ok(StorageResult::Completed)
        }
        StorageOperation::Report { request, data } => Ok(StorageResult::Report(
            crate::domain::calculate(request, &data.as_data()),
        )),
    }
}

/// Returns true only for the latest request and currently open budget/view generation.
pub fn response_is_current(
    response: &StorageResponse,
    expected_id: RequestId,
    generation: Generation,
) -> bool {
    matches!(response,StorageResponse::Completed{id,generation:g,..} if *id==expected_id && *g==generation)
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
                operation: StorageOperation::Health,
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
                operation: StorageOperation::Health
            }),
            Err(WorkerError::Shutdown)
        ));
    }
}
