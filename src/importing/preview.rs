use super::{deduplication::CandidateClassification, source::ImportedTransaction};

#[derive(Clone, Debug, Eq, PartialEq)]
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
impl ImportPreview {
    pub fn decide(&mut self, index: usize, decision: ReviewDecision) -> bool {
        self.candidates.get_mut(index).is_some_and(|c| {
            c.decision = decision;
            true
        })
    }
}
