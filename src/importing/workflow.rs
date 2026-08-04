//! Durable import workflow state and review policies. Parsing produces inert previews;
//! staging persists batches/candidates separately from financial transactions; applying
//! is the first workflow step allowed to create or update financial records.
use super::{
    deduplication::{CandidateClassification, MatchEvidence},
    preview::ReviewDecision,
    source::{ImportedTransaction, SourceAccount, SourceLocation},
};
use crate::domain::{AccountId, BudgetMonth, CategoryId, ImportBatchId, TransactionId};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportWorkflowState {
    SelectingFile,
    DetectingFormat,
    ConfiguringCsv,
    Parsing,
    Staging,
    Reviewing,
    Applying,
    Archiving,
    Completed,
    Failed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShutdownOperation {
    CancelImmediately,
    PromptUser,
    WaitForSafePoint,
    BlockUntilFinished,
}

impl ImportWorkflowState {
    #[must_use]
    pub const fn can_transition_to(self, next: Self) -> bool {
        use ImportWorkflowState as S;
        matches!(
            (self, next),
            (S::SelectingFile, S::DetectingFormat | S::Failed)
                | (
                    S::DetectingFormat,
                    S::ConfiguringCsv | S::Parsing | S::Failed
                )
                | (S::ConfiguringCsv, S::Parsing | S::SelectingFile | S::Failed)
                | (S::Parsing, S::Staging | S::ConfiguringCsv | S::Failed)
                | (S::Staging, S::Reviewing | S::Failed)
                | (S::Reviewing, S::Applying | S::ConfiguringCsv | S::Failed)
                | (S::Applying, S::Archiving | S::Failed)
                | (S::Archiving, S::Completed | S::Failed)
                | (
                    S::Failed,
                    S::SelectingFile
                        | S::ConfiguringCsv
                        | S::Parsing
                        | S::Staging
                        | S::Reviewing
                        | S::Archiving
                )
        )
    }
    #[must_use]
    pub const fn is_cancellable(self) -> bool {
        !matches!(self, Self::Applying | Self::Archiving | Self::Completed)
    }
    #[must_use]
    pub const fn failure_is_resumable(self) -> bool {
        matches!(
            self,
            Self::ConfiguringCsv
                | Self::Parsing
                | Self::Staging
                | Self::Reviewing
                | Self::Archiving
        )
    }
    #[must_use]
    pub const fn shutdown_operation(self) -> ShutdownOperation {
        match self {
            Self::SelectingFile
            | Self::DetectingFormat
            | Self::ConfiguringCsv
            | Self::Parsing
            | Self::Staging => ShutdownOperation::CancelImmediately,
            Self::Reviewing => ShutdownOperation::PromptUser,
            Self::Applying => ShutdownOperation::WaitForSafePoint,
            Self::Archiving => ShutdownOperation::BlockUntilFinished,
            Self::Completed | Self::Failed => ShutdownOperation::CancelImmediately,
        }
    }
}

pub type BatchState = ImportWorkflowState;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct StatementDetails {
    pub account: Option<SourceAccount>,
    pub currency: Option<String>,
    pub account_mismatch: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct StoredImportBatch {
    pub id: ImportBatchId,
    pub workflow_state: ImportWorkflowState,
    pub source_name: String,
    pub source_account_identifier: Option<String>,
    pub selected_destination_account: AccountId,
    pub archive_path: Option<String>,
    pub applied_generation: Option<u64>,
    pub applied_revision: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MatchCandidateProjection {
    pub transaction_id: TransactionId,
    pub classification: CandidateClassification,
    pub evidence: Vec<MatchEvidence>,
    pub explanation: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct StoredImportCandidate {
    pub batch_id: ImportBatchId,
    pub sort_order: u32,
    /// Immutable parser output. Review edits must only change proposed fields below.
    pub original: ImportedTransaction,
    pub proposed_payee: Option<String>,
    pub proposed_category: Option<CategoryId>,
    pub proposed_memo: Option<String>,
    pub duplicate_classification: CandidateClassification,
    pub match_candidates: Vec<MatchCandidateProjection>,
    pub warnings: Vec<String>,
    pub decision: ReviewDecision,
    pub source_account_identifier: Option<String>,
    pub selected_destination_account: AccountId,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DecisionError {
    ExactDuplicateOverrideRequired,
    UnsafeBulkOperation,
    MatchTargetRequired,
    AllCandidatesRequireDecision,
    InvalidDestinationReference,
    StaleBudgetRevision,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExactDuplicateOverride(());
impl ExactDuplicateOverride {
    #[must_use]
    pub const fn confirmed() -> Self {
        Self(())
    }
}

pub fn validate_decision(
    classification: CandidateClassification,
    decision: &ReviewDecision,
    override_confirmation: Option<ExactDuplicateOverride>,
) -> Result<(), DecisionError> {
    if matches!(decision, ReviewDecision::Accept)
        && classification == CandidateClassification::ExactImportIdDuplicate
        && override_confirmation.is_none()
    {
        return Err(DecisionError::ExactDuplicateOverrideRequired);
    }
    if matches!(decision, ReviewDecision::Match(target) if target.trim().is_empty()) {
        return Err(DecisionError::MatchTargetRequired);
    }
    Ok(())
}

pub fn apply_bulk(
    decisions: &mut [(CandidateClassification, ReviewDecision)],
    decision: ReviewDecision,
) -> Result<(), DecisionError> {
    if decision == ReviewDecision::Accept
        && decisions
            .iter()
            .any(|(class, _)| *class != CandidateClassification::New)
    {
        return Err(DecisionError::UnsafeBulkOperation);
    }
    if matches!(decision, ReviewDecision::Match(_)) {
        return Err(DecisionError::UnsafeBulkOperation);
    }
    for (_, current) in decisions {
        *current = decision.clone();
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImportInvalidationPlan {
    pub registers: Vec<AccountId>,
    pub balances: Vec<AccountId>,
    pub affected_budget_months: Vec<BudgetMonth>,
    pub inbox: bool,
    pub reports: bool,
    pub search: bool,
    pub targets: bool,
    pub inspector_projections: Vec<TransactionId>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApplyCandidateDecision {
    pub candidate_index: u32,
    pub decision: ReviewDecision,
    pub destination_account: AccountId,
    pub expected_budget_generation: u64,
    pub expected_budget_revision: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewedImportCommand {
    pub batch_id: ImportBatchId,
    pub decisions: Vec<ApplyCandidateDecision>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RowDiagnostic {
    pub location: SourceLocation,
    pub field: Option<String>,
    pub message: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParsePreview {
    pub details: StatementDetails,
    pub rows: Vec<ImportedTransaction>,
    pub diagnostics: Vec<RowDiagnostic>,
}

impl ParsePreview {
    #[must_use]
    pub fn read_only_from_statement(statement: super::source::ImportedStatement) -> Self {
        Self {
            details: StatementDetails {
                account: statement.account,
                currency: statement.currency,
                account_mismatch: None,
            },
            rows: statement.transactions,
            diagnostics: Vec::new(),
        }
    }
}

#[must_use]
pub const fn parsing_creates_financial_records() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explicit_state_machine_and_lifecycle_classification() {
        assert!(
            ImportWorkflowState::SelectingFile
                .can_transition_to(ImportWorkflowState::DetectingFormat)
        );
        assert!(
            !ImportWorkflowState::SelectingFile.can_transition_to(ImportWorkflowState::Reviewing)
        );
        assert!(ImportWorkflowState::Parsing.is_cancellable());
        assert_eq!(
            ImportWorkflowState::Reviewing.shutdown_operation(),
            ShutdownOperation::PromptUser
        );
        assert_eq!(
            ImportWorkflowState::Applying.shutdown_operation(),
            ShutdownOperation::WaitForSafePoint
        );
        assert_eq!(
            ImportWorkflowState::Archiving.shutdown_operation(),
            ShutdownOperation::BlockUntilFinished
        );
        assert!(ImportWorkflowState::Archiving.failure_is_resumable());
    }
    #[test]
    fn exact_duplicates_need_individual_override() {
        assert_eq!(
            validate_decision(
                CandidateClassification::ExactImportIdDuplicate,
                &ReviewDecision::Accept,
                None
            ),
            Err(DecisionError::ExactDuplicateOverrideRequired)
        );
        assert!(
            validate_decision(
                CandidateClassification::ExactImportIdDuplicate,
                &ReviewDecision::Accept,
                Some(ExactDuplicateOverride::confirmed())
            )
            .is_ok()
        );
        let mut rows = [
            (CandidateClassification::New, ReviewDecision::Pending),
            (
                CandidateClassification::PossibleManualMatch,
                ReviewDecision::Pending,
            ),
        ];
        assert_eq!(
            apply_bulk(&mut rows, ReviewDecision::Accept),
            Err(DecisionError::UnsafeBulkOperation)
        );
        apply_bulk(&mut rows, ReviewDecision::Ignore).unwrap();
    }
}
