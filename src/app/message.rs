use crate::storage::worker::{Generation, RequestId};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OperationError {
    Cancelled,
    Storage(String),
    Import(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WorkerPayload {
    Loaded,
    Progress(u8),
    Failed(OperationError),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkerMessage {
    pub request_id: RequestId,
    pub generation: Generation,
    pub payload: WorkerPayload,
}
