//! Storage-to-application mutation protocol.  It deliberately contains domain values only.

use crate::{
    app::{
        command::{CommandId, CorrelationId, FinancialCommand, FocusRestorationId},
        view_invalidation::ViewInvalidations,
    },
    domain::{
        AccountId, CategoryId, ImportBatchId, PayeeId, ReconciliationId, ScheduledTransactionId,
        TargetId, TransactionId,
    },
};

/// Stable purpose tag for read requests. Together with the request id and generation this lets
/// consumers reject a late response without inspecting its payload.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum RequestPurpose {
    AccountTree,
    AccountHeader,
    AccountRegisterPage,
    AllTransactionsPage,
    CategoryCatalog,
    CategoryGoalDetails,
    MerchantRules,
    Report,
    ImportPreview,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AffectedEntityId {
    Account(AccountId),
    Category(CategoryId),
    Payee(PayeeId),
    Transaction(TransactionId),
    ImportBatch(ImportBatchId),
    Reconciliation(ReconciliationId),
    Target(TargetId),
    Schedule(ScheduledTransactionId),
    Assignment {
        category: CategoryId,
        month: crate::domain::BudgetMonth,
    },
}

/// An inverse command is preferred.  Opaque data is reserved for lossless snapshots which need a
/// future, version-aware undo handler and must never be interpreted by the UI.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UndoData {
    Command(FinancialCommand),
    Opaque(Vec<u8>),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MutationResult {
    pub command_id: CommandId,
    pub correlation_id: CorrelationId,
    pub operation_label: &'static str,
    pub affected_entity_ids: Vec<AffectedEntityId>,
    pub undo: Option<UndoData>,
    pub invalidations: ViewInvalidations,
    pub navigation: Option<String>,
    pub focus_restoration: Option<FocusRestorationId>,
    pub notice: Option<String>,
}
