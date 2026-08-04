//! Reconciliation snapshots and checked calculations.
use super::*;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use time::OffsetDateTime;

/// Lifecycle state. Invalidity is retained rather than deleting historical input.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ReconciliationState {
    NotReconciling,
    EnteringStatement,
    Active,
    ReviewingAdjustment,
    Completing,
    Completed,
    PotentiallyInvalid,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Reconciliation {
    pub id: ReconciliationId,
    pub budget_id: BudgetId,
    pub account_id: AccountId,
    pub statement_date: StatementDate,
    pub ending_balance: Money,
    /// Cleared ledger balance captured at completion (or the current preview while active).
    pub calculated_cleared_balance: Money,
    /// `ending_balance - calculated_cleared_balance`.
    pub difference: Money,
    pub included_transaction_ids: Vec<TransactionId>,
    pub state: ReconciliationState,
    pub created_at: OffsetDateTime,
    pub completed_at: Option<OffsetDateTime>,
    pub invalidated_at: Option<OffsetDateTime>,
}

#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub enum ReconciliationCalculationError {
    #[error("reconciliation arithmetic overflow")]
    Overflow,
}

pub fn cleared_balance<'a>(
    transactions: impl IntoIterator<Item = &'a Transaction>,
) -> Result<Money, ReconciliationCalculationError> {
    transactions
        .into_iter()
        .filter(|t| {
            !t.archived
                && !t.voided
                && matches!(t.clearance, Clearance::Cleared | Clearance::Reconciled)
        })
        .try_fold(Money::ZERO, |sum, t| sum.checked_add(t.amount))
        .map_err(|_| ReconciliationCalculationError::Overflow)
}

pub fn reconciliation_difference(
    ending: Money,
    cleared: Money,
) -> Result<Money, ReconciliationCalculationError> {
    ending
        .checked_sub(cleared)
        .map_err(|_| ReconciliationCalculationError::Overflow)
}

/// The visible adjustment required under the central `statement - cleared` convention.
pub fn adjustment_amount(
    ending: Money,
    cleared: Money,
) -> Result<Money, ReconciliationCalculationError> {
    reconciliation_difference(ending, cleared)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn transaction(amount: i64) -> Transaction {
        Transaction {
            id: TransactionId::new(),
            budget_id: BudgetId::new(),
            account_id: AccountId::new(),
            date: TransactionDate(time::macros::date!(2026 - 01 - 01)),
            payee_id: None,
            amount: Money::from_minor_units(amount),
            memo: None,
            clearance: Clearance::Cleared,
            approval: Approval::Approved,
            body: TransactionBody::OpeningBalance { category_id: None },
            archived: false,
            voided: false,
        }
    }
    #[test]
    fn signs_and_adjustment_follow_one_convention() {
        assert_eq!(
            cleared_balance([&transaction(120), &transaction(-20)]).unwrap(),
            Money::from_minor_units(100)
        );
        assert_eq!(
            reconciliation_difference(Money::from_minor_units(130), Money::from_minor_units(100))
                .unwrap(),
            Money::from_minor_units(30)
        );
        assert_eq!(
            reconciliation_difference(Money::from_minor_units(-130), Money::from_minor_units(-100))
                .unwrap(),
            Money::from_minor_units(-30)
        );
        let adjustment =
            adjustment_amount(Money::from_minor_units(-130), Money::from_minor_units(-100))
                .unwrap();
        assert_eq!(
            reconciliation_difference(
                Money::from_minor_units(-130),
                Money::from_minor_units(-100)
                    .checked_add(adjustment)
                    .unwrap()
            )
            .unwrap(),
            Money::ZERO
        );
    }
    #[test]
    fn overflow_is_reported() {
        assert!(cleared_balance([&transaction(i64::MAX), &transaction(1)]).is_err());
        assert!(
            reconciliation_difference(
                Money::from_minor_units(i64::MAX),
                Money::from_minor_units(-1)
            )
            .is_err()
        );
    }
}
