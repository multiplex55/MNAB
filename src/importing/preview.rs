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
    pub auto_categorized: bool,
    pub matched_merchant_rule: Option<String>,
}
impl ImportProposal {
    #[must_use]
    pub fn included_by_default(&self) -> bool {
        self.duplicate_state != CandidateClassification::ExactDuplicate
            && self.decision != ReviewDecision::Ignore
    }
    /// Runs after duplicate classification and before staging. The immutable `source`
    /// remains available for review/audit and a match never accepts or approves the row.
    pub fn apply_merchant_rule(
        &mut self,
        book: &crate::service::merchant_rule_service::MerchantRuleBook,
        account: AccountId,
    ) -> Result<bool, crate::service::merchant_rule_service::RuleEvaluationError> {
        let merchant = self
            .proposed_payee
            .as_deref()
            .or(self.source.payee.as_deref())
            .unwrap_or("");
        let context = crate::service::merchant_rule_service::ImportRuleContext {
            merchant,
            account_id: account,
            account_group_id: None,
            amount_minor_units: self.source.amount.minor_units(),
            memo: self
                .proposed_memo
                .as_deref()
                .or(self.source.memo.as_deref()),
            import_source: match self.source.location {
                super::source::SourceLocation::CsvRow(_) => "csv",
                super::source::SourceLocation::OfxTransaction { .. } => "ofx",
            },
        };
        let proposal = book.evaluate(&context)?;
        if proposal.matched_rule_ids.is_empty() {
            return Ok(false);
        }
        if let Some((_, snapshot)) = proposal.payee {
            self.proposed_payee = Some(snapshot);
        }
        if let Some(category) = proposal.category {
            self.proposed_category = Some(category.to_string());
        }
        if let Some(memo) = proposal.memo {
            self.proposed_memo = Some(memo);
        }
        self.selected_destination_account = Some(account);
        self.auto_categorized = true;
        self.matched_merchant_rule = proposal.matched_rule_ids.first().map(ToString::to_string);
        self.decision = ReviewDecision::Pending;
        Ok(true)
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
        candidate.auto_categorized = false;
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
