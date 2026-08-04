//! Durable import state and review policies.  A batch is persisted in `ParsedStaged`
//! before it can move to `AwaitingReview`; the UI never owns the authoritative copy.
use super::{
    deduplication::CandidateClassification, preview::ReviewDecision, source::SourceAccount,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BatchState {
    Selected,
    ParsedStaged,
    AwaitingReview,
    Applied,
    ArchivePending,
    Archived,
    Failed,
}
impl BatchState {
    #[must_use]
    pub const fn can_transition_to(self, next: Self) -> bool {
        matches!(
            (self, next),
            (Self::Selected, Self::ParsedStaged | Self::Failed)
                | (Self::ParsedStaged, Self::AwaitingReview | Self::Failed)
                | (Self::AwaitingReview, Self::Applied | Self::Failed)
                | (Self::Applied, Self::ArchivePending | Self::Archived)
                | (Self::ArchivePending, Self::Archived | Self::Failed)
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct StatementDetails {
    pub account: Option<SourceAccount>,
    pub currency: Option<String>,
    pub account_mismatch: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DecisionError {
    ExactDuplicateOverrideRequired,
    UnsafeBulkOperation,
    MatchTargetRequired,
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
        && classification == CandidateClassification::ExactDuplicate
        && override_confirmation.is_none()
    {
        return Err(DecisionError::ExactDuplicateOverrideRequired);
    }
    if matches!(decision, ReviewDecision::Match(target) if target.trim().is_empty()) {
        return Err(DecisionError::MatchTargetRequired);
    }
    Ok(())
}

/// Bulk acceptance is limited to new rows. Duplicates and suggested matches must
/// be considered individually; bulk ignore is safe for every classification.
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn state_machine_prevents_review_before_staging() {
        assert!(!BatchState::Selected.can_transition_to(BatchState::AwaitingReview));
        assert!(BatchState::Selected.can_transition_to(BatchState::ParsedStaged));
        assert!(BatchState::ParsedStaged.can_transition_to(BatchState::AwaitingReview));
        assert!(!BatchState::Archived.can_transition_to(BatchState::Applied));
    }
    #[test]
    fn exact_duplicates_need_individual_override() {
        assert_eq!(
            validate_decision(
                CandidateClassification::ExactDuplicate,
                &ReviewDecision::Accept,
                None
            ),
            Err(DecisionError::ExactDuplicateOverrideRequired)
        );
        assert!(
            validate_decision(
                CandidateClassification::ExactDuplicate,
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
