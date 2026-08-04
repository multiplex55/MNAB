//! Derived, disposable inbox projection.
//!
//! Financial facts are always re-read from their owning tables.  An inbox row is
//! therefore never persisted.  Only a dismissal of a non-financial operation
//! failure may be durable.
use std::collections::{BTreeMap, BTreeSet};
use time::{Date, Duration};

use crate::domain::{
    AccountId, BudgetMonth, CategoryId, ImportBatchId, ScheduledOccurrenceId, TargetId,
    TransactionId,
};

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum InboxItemId {
    StagedImport(ImportBatchId),
    Transaction(TransactionId),
    DuplicateCandidate {
        batch_id: ImportBatchId,
        candidate_id: String,
    },
    Occurrence(ScheduledOccurrenceId),
    Reconciliation(AccountId),
    Overspent {
        category_id: CategoryId,
        month: BudgetMonth,
    },
    Underfunded {
        target_id: TargetId,
        month: BudgetMonth,
    },
    FailedOperation(String),
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum InboxReason {
    FailedOperation,
    StagedImport,
    PossibleDuplicate,
    Unapproved,
    Uncategorized,
    Overdue,
    DueSoon,
    StaleUncleared,
    ReconciliationDue,
    Overspent,
    Underfunded,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InboxItem {
    pub id: InboxItemId,
    pub reasons: Vec<InboxReason>,
    pub title: String,
    pub related_entities: Vec<String>,
    pub amount_cents: Option<i64>,
    pub date: Option<Date>,
    pub recommended_resolution: String,
    pub actions: Vec<crate::app::command::InboxAction>,
    pub dismissible: bool,
    pub reversible: bool,
}

impl InboxItem {
    /// Merge reasons only when the immutable source identity is identical.
    pub fn merge(mut self, other: Self) -> Self {
        debug_assert_eq!(self.id, other.id);
        self.reasons.extend(other.reasons);
        self.reasons.sort_unstable();
        self.reasons.dedup();
        self.actions.extend(other.actions);
        self.actions.sort_by_key(|a| *a as u8);
        self.actions.dedup();
        self.related_entities.extend(other.related_entities);
        self.related_entities.sort();
        self.related_entities.dedup();
        self
    }
}

#[allow(clippy::struct_field_names)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InboxPolicy {
    pub uncleared_age_days: i64,
    pub due_soon_days: i64,
    pub reconciliation_cadence_days: i64,
}
impl Default for InboxPolicy {
    fn default() -> Self {
        Self {
            uncleared_age_days: 30,
            due_soon_days: 7,
            reconciliation_cadence_days: 30,
        }
    }
}
impl InboxPolicy {
    pub fn occurrence_reason(self, today: Date, occurrence: Date) -> Option<InboxReason> {
        if occurrence < today {
            Some(InboxReason::Overdue)
        } else if occurrence <= today + Duration::days(self.due_soon_days) {
            Some(InboxReason::DueSoon)
        } else {
            None
        }
    }
    pub fn stale_uncleared(self, today: Date, transaction: Date) -> bool {
        transaction <= today - Duration::days(self.uncleared_age_days)
    }
    pub fn reconciliation_due(
        self,
        today: Date,
        last_completed: Option<Date>,
        has_post_reconciliation_activity: bool,
        has_non_opening_activity: bool,
    ) -> bool {
        match last_completed {
            Some(last) => {
                has_post_reconciliation_activity
                    && last <= today - Duration::days(self.reconciliation_cadence_days)
            }
            None => has_non_opening_activity,
        }
    }
}

pub fn merge_items(items: impl IntoIterator<Item = InboxItem>) -> Vec<InboxItem> {
    let mut merged = BTreeMap::new();
    for item in items {
        merged
            .entry(item.id.clone())
            .and_modify(|old: &mut InboxItem| *old = old.clone().merge(item.clone()))
            .or_insert(item);
    }
    merged.into_values().collect()
}

/// Query boundary implemented by storage. Each method reads its authoritative
/// source; implementations must not materialize financial state in an inbox table.
pub trait InboxSource {
    type Error;
    fn staged_imports(&self) -> Result<Vec<InboxItem>, Self::Error>;
    fn transaction_attention(&self) -> Result<Vec<InboxItem>, Self::Error>;
    fn duplicate_candidates(&self) -> Result<Vec<InboxItem>, Self::Error>;
    fn due_occurrences(
        &self,
        today: Date,
        due_through: Date,
    ) -> Result<Vec<InboxItem>, Self::Error>;
    fn stale_uncleared(&self, on_or_before: Date) -> Result<Vec<InboxItem>, Self::Error>;
    fn reconciliation_due(
        &self,
        today: Date,
        cadence_days: i64,
    ) -> Result<Vec<InboxItem>, Self::Error>;
    fn overspent_categories(&self) -> Result<Vec<InboxItem>, Self::Error>;
    fn underfunded_targets(&self) -> Result<Vec<InboxItem>, Self::Error>;
    fn failed_operations(&self) -> Result<Vec<InboxItem>, Self::Error>;
}

pub fn query_all<S: InboxSource>(
    source: &S,
    today: Date,
    policy: InboxPolicy,
) -> Result<Vec<InboxItem>, S::Error> {
    let mut rows = Vec::new();
    rows.extend(source.staged_imports()?);
    rows.extend(source.transaction_attention()?);
    rows.extend(source.duplicate_candidates()?);
    rows.extend(source.due_occurrences(today, today + Duration::days(policy.due_soon_days))?);
    rows.extend(source.stale_uncleared(today - Duration::days(policy.uncleared_age_days))?);
    rows.extend(source.reconciliation_due(today, policy.reconciliation_cadence_days)?);
    rows.extend(source.overspent_categories()?);
    rows.extend(source.underfunded_targets()?);
    rows.extend(source.failed_operations()?);
    Ok(merge_items(rows))
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct InboxCounts {
    pub total: usize,
    pub by_reason: BTreeMap<InboxReason, usize>,
}
pub fn counts(items: &[InboxItem]) -> InboxCounts {
    let mut result = InboxCounts {
        total: items.len(),
        by_reason: BTreeMap::new(),
    };
    for item in items {
        for reason in &item.reasons {
            *result.by_reason.entry(*reason).or_default() += 1;
        }
    }
    result
}
pub fn review_window(
    items: &[InboxItem],
    current: Option<&InboxItemId>,
    look_ahead: usize,
) -> Vec<InboxItem> {
    let start = current
        .and_then(|id| items.iter().position(|i| &i.id == id))
        .unwrap_or(0);
    items
        .iter()
        .skip(start)
        .take(look_ahead.saturating_add(1))
        .cloned()
        .collect()
}
pub fn inbox_zero(items: &[InboxItem], hidden_or_dismissed: &BTreeSet<InboxItemId>) -> bool {
    !items
        .iter()
        .any(|item| !hidden_or_dismissed.contains(&item.id))
}

/// Re-query after every command; identity, never an obsolete numeric index, drives advancement.
pub fn advance_after_resolution(
    new_items: &[InboxItem],
    resolved: &InboxItemId,
) -> Option<InboxItemId> {
    new_items
        .iter()
        .find(|item| &item.id != resolved)
        .map(|item| item.id.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::date;
    fn item(id: InboxItemId, reason: InboxReason) -> InboxItem {
        InboxItem {
            id,
            reasons: vec![reason],
            title: "x".into(),
            related_entities: vec![],
            amount_cents: None,
            date: None,
            recommended_resolution: "fix".into(),
            actions: vec![],
            dismissible: false,
            reversible: true,
        }
    }
    #[test]
    fn stable_identity_and_merging() {
        let id = InboxItemId::Transaction(TransactionId::new());
        let v = merge_items([
            item(id.clone(), InboxReason::Unapproved),
            item(id.clone(), InboxReason::Uncategorized),
        ]);
        assert_eq!(v.len(), 1);
        assert_eq!(
            v[0].reasons,
            vec![InboxReason::Unapproved, InboxReason::Uncategorized]
        );
    }
    #[test]
    fn boundaries_and_reconciliation_rules() {
        let p = InboxPolicy::default();
        let today = date!(2026 - 08 - 04);
        assert_eq!(
            p.occurrence_reason(today, date!(2026 - 08 - 11)),
            Some(InboxReason::DueSoon)
        );
        assert!(p.stale_uncleared(today, date!(2026 - 07 - 05)));
        assert!(!p.reconciliation_due(today, Some(date!(2026 - 06 - 01)), false, true));
        assert!(p.reconciliation_due(today, Some(date!(2026 - 06 - 01)), true, true));
        assert!(!p.reconciliation_due(today, None, true, false));
        assert!(p.reconciliation_due(today, None, false, true));
    }
    #[test]
    fn counts_window_advancement_and_zero_are_consistent() {
        let a = item(
            InboxItemId::FailedOperation("a".into()),
            InboxReason::FailedOperation,
        );
        let b = item(
            InboxItemId::FailedOperation("b".into()),
            InboxReason::FailedOperation,
        );
        let all = vec![a.clone(), b.clone()];
        assert_eq!(counts(&all).total, 2);
        assert_eq!(review_window(&all, Some(&a.id), 1), all);
        assert_eq!(
            advance_after_resolution(std::slice::from_ref(&b), &a.id),
            Some(b.id.clone())
        );
        assert!(!inbox_zero(&all, &BTreeSet::new()));
        assert!(inbox_zero(&all, &BTreeSet::from([a.id, b.id])));
    }
}
