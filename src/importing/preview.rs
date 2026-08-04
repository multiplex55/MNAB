use super::{deduplication::CandidateClassification, source::ImportedTransaction};
use crate::domain::{AccountId, TransactionId};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ReviewDecision {
    Pending,
    Accept,
    Ignore,
    Match(String),
}
#[derive(Clone, Debug)]
pub struct ImportProposal {
    /// Immutable parser output; edits are held in the proposed fields below.
    pub source: ImportedTransaction,
    pub proposed_payee: Option<String>,
    pub proposed_category: Option<String>,
    pub proposed_memo: Option<String>,
    pub duplicate_state: CandidateClassification,
    pub decision: ReviewDecision,
    pub warnings: Vec<String>,
    pub match_candidates: Vec<TransactionId>,
    pub selected_destination_account: Option<AccountId>,
}
impl ImportProposal {
    #[must_use]
    pub fn included_by_default(&self) -> bool {
        self.duplicate_state != CandidateClassification::ExactDuplicate
            && self.decision != ReviewDecision::Ignore
    }
}

#[derive(Clone, Debug, Default)]
pub struct ImportPreview {
    pub candidates: Vec<ImportProposal>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImportReviewWorkspace {
    pub selected: Vec<usize>,
    pub default_destination_account: AccountId,
}
impl ImportReviewWorkspace {
    #[must_use]
    pub fn inspect_explanation(candidate: &ImportProposal) -> Option<&str> {
        candidate.warnings.first().map(String::as_str)
    }
    pub fn override_payee(candidate: &mut ImportProposal, payee: impl Into<String>) {
        candidate.proposed_payee = Some(payee.into());
    }
    pub fn override_category(candidate: &mut ImportProposal, category: impl Into<String>) {
        candidate.proposed_category = Some(category.into());
    }
    pub fn override_memo(candidate: &mut ImportProposal, memo: impl Into<String>) {
        candidate.proposed_memo = Some(memo.into());
    }
    pub fn resolve_duplicate(candidate: &mut ImportProposal, decision: ReviewDecision) {
        candidate.decision = decision;
    }
    pub fn resolve_account_mismatch(candidate: &mut ImportProposal, destination: AccountId) {
        candidate.selected_destination_account = Some(destination);
    }
}

impl ImportPreview {
    pub fn decide(&mut self, index: usize, decision: ReviewDecision) -> bool {
        self.candidates.get_mut(index).is_some_and(|c| {
            c.decision = decision;
            true
        })
    }
}
