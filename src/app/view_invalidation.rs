use std::collections::BTreeSet;

use crate::domain::{AccountId, BudgetMonth, ImportBatchId, TargetId, TransactionId};

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum ViewInvalidation {
    Accounts,
    BudgetMonth(BudgetMonth),
    AccountRegister(AccountId),
    AllAccountRegisters,
    Inbox,
    Reports,
    Targets,
    Schedules,
    Search,
    Inspectors,
    TransactionInspector(TransactionId),
    ImportReview(ImportBatchId),
    Reconciliation(AccountId),
    Target(TargetId),
    /// The changed month and every materialized later month (rollover is cumulative).
    BudgetRolloverFrom(BudgetMonth),
    /// Payee/category/account choices used by editors and search suggestions.
    LookupData,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ViewInvalidations(BTreeSet<ViewInvalidation>);

impl ViewInvalidations {
    pub fn insert(&mut self, value: ViewInvalidation) {
        if value == ViewInvalidation::AllAccountRegisters {
            self.0
                .retain(|v| !matches!(v, ViewInvalidation::AccountRegister(_)));
        } else if matches!(value, ViewInvalidation::AccountRegister(_))
            && self.0.contains(&ViewInvalidation::AllAccountRegisters)
        {
            return;
        }
        self.0.insert(value);
    }
    pub fn merge(&mut self, other: Self) {
        for value in other.0 {
            self.insert(value);
        }
    }
    pub fn iter(&self) -> impl Iterator<Item = &ViewInvalidation> {
        self.0.iter()
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn clear(&mut self) {
        self.0.clear();
    }
}

/// Coalesces mutation invalidations until the next scheduling pass. A query key can
/// only be queued once, even when several commands invalidate it in one frame.
#[derive(Clone, Debug, Default)]
pub struct QueryScheduler {
    pending: ViewInvalidations,
}

impl QueryScheduler {
    pub fn invalidate(&mut self, invalidations: ViewInvalidations) {
        self.pending.merge(invalidations);
    }

    pub fn drain(&mut self) -> ViewInvalidations {
        std::mem::take(&mut self.pending)
    }

    pub fn pending(&self) -> &ViewInvalidations {
        &self.pending
    }
}

impl FromIterator<ViewInvalidation> for ViewInvalidations {
    fn from_iter<T: IntoIterator<Item = ViewInvalidation>>(iter: T) -> Self {
        let mut result = Self::default();
        for value in iter {
            result.insert(value);
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_registers_bounds_specific_registers() {
        let mut values: ViewInvalidations = [
            ViewInvalidation::AccountRegister(AccountId::new()),
            ViewInvalidation::Accounts,
        ]
        .into_iter()
        .collect();
        values.insert(ViewInvalidation::AllAccountRegisters);
        values.insert(ViewInvalidation::AccountRegister(AccountId::new()));
        assert_eq!(
            values
                .iter()
                .filter(|v| matches!(v, ViewInvalidation::AccountRegister(_)))
                .count(),
            0
        );
        assert_eq!(values.iter().count(), 2);
    }

    #[test]
    fn scheduling_deduplicates_and_coalesces() {
        let account = AccountId::new();
        let mut scheduler = QueryScheduler::default();
        scheduler.invalidate(
            [ViewInvalidation::AccountRegister(account)]
                .into_iter()
                .collect(),
        );
        scheduler.invalidate(
            [
                ViewInvalidation::AccountRegister(account),
                ViewInvalidation::Reports,
            ]
            .into_iter()
            .collect(),
        );
        assert_eq!(scheduler.drain().iter().count(), 2);
        assert!(scheduler.pending().is_empty());
    }
}
