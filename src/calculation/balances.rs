//! Transaction-derived account balances and deterministic register ordering.
use crate::domain::{Clearance, Money, MoneyError, Transaction, TransactionId};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct AccountBalances {
    pub cleared: Money,
    pub uncleared: Money,
    pub working: Money,
    pub reconciled: Money,
}

fn contributes(transaction: &Transaction) -> bool {
    !transaction.archived && !transaction.voided
}

pub fn derive<'a>(
    transactions: impl IntoIterator<Item = &'a Transaction>,
) -> Result<AccountBalances, MoneyError> {
    let mut result = AccountBalances::default();
    for transaction in transactions.into_iter().filter(|t| contributes(t)) {
        result.working = result.working.checked_add(transaction.amount)?;
        match transaction.clearance {
            Clearance::Uncleared => {
                result.uncleared = result.uncleared.checked_add(transaction.amount)?;
            }
            Clearance::Cleared => {
                result.cleared = result.cleared.checked_add(transaction.amount)?;
            }
            Clearance::Reconciled => {
                result.cleared = result.cleared.checked_add(transaction.amount)?;
                result.reconciled = result.reconciled.checked_add(transaction.amount)?;
            }
        }
    }
    Ok(result)
}

/// Stable sequence is date then immutable transaction ID.
pub fn running_balances<'a>(
    transactions: impl IntoIterator<Item = &'a Transaction>,
) -> Result<Vec<(TransactionId, Money)>, MoneyError> {
    let mut rows: Vec<_> = transactions
        .into_iter()
        .filter(|t| contributes(t))
        .collect();
    rows.sort_by_key(|t| (t.date.0, t.id));
    let mut balance = Money::ZERO;
    rows.into_iter()
        .map(|t| {
            balance = balance.checked_add(t.amount)?;
            Ok((t.id, balance))
        })
        .collect()
}
