use std::path::PathBuf;

use crate::domain::BudgetId;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionSummary {
    pub budget_name: String,
    pub account_count: usize,
}

/// The single committed budget identity. A session does not contain a database
/// handle; the matching worker has exclusive ownership of the writable handle.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BudgetSession {
    pub budget_id: BudgetId,
    pub database_path: PathBuf,
    pub schema_version: u32,
    pub summary: SessionSummary,
}
