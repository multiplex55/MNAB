//! Continuous, account-backed category goal projections.
//!
//! These values are deliberately derived from ledger balances. In particular,
//! `uncategorized` is never an allocation and must not be written to storage.

use crate::domain::{AccountId, CategoryId, Money, MoneyError};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GoalBalance {
    pub category_id: CategoryId,
    pub current: Money,
    pub target: Money,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GoalProgress {
    pub current: Money,
    pub remaining: Money,
    /// Completion in basis points (10_000 is 100%). Values above 100% are capped.
    pub completion_basis_points: u16,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountGoalSummary {
    pub account_id: AccountId,
    pub account_balance: Money,
    pub goals: Vec<GoalBalance>,
    pub uncategorized_balance: Money,
    pub overcommitted_by: Money,
}

#[must_use]
pub fn progress(current: Money, target: Money) -> GoalProgress {
    let target_cents = target.minor_units().max(0);
    let current_cents = current.minor_units().max(0);
    let remaining = target_cents.saturating_sub(current_cents);
    let percentage = if target_cents == 0 {
        10_000
    } else {
        (i128::from(current_cents) * 10_000 / i128::from(target_cents)).min(10_000) as u16
    };
    GoalProgress {
        current,
        remaining: Money::from_minor_units(remaining),
        completion_basis_points: percentage,
    }
}

/// Projects an account summary, ignoring zero and negative category balances.
/// The caller supplies only goals belonging to `account_id`.
pub fn account_summary(
    account_id: AccountId,
    account_balance: Money,
    goals: impl IntoIterator<Item = GoalBalance>,
) -> Result<AccountGoalSummary, MoneyError> {
    let goals: Vec<_> = goals
        .into_iter()
        .filter(|goal| goal.current > Money::ZERO)
        .collect();
    let total = goals
        .iter()
        .try_fold(Money::ZERO, |sum, goal| sum.checked_add(goal.current))?;
    let uncategorized_balance = account_balance.checked_sub(total)?;
    let overcommitted_by = if uncategorized_balance < Money::ZERO {
        uncategorized_balance.checked_neg()?
    } else {
        Money::ZERO
    };
    Ok(AccountGoalSummary {
        account_id,
        account_balance,
        goals,
        uncategorized_balance,
        overcommitted_by,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calculates_progress_remaining_and_caps_percentage() {
        let result = progress(
            Money::from_minor_units(2_500),
            Money::from_minor_units(10_000),
        );
        assert_eq!(result.remaining, Money::from_minor_units(7_500));
        assert_eq!(result.completion_basis_points, 2_500);
        assert_eq!(
            progress(
                Money::from_minor_units(12_000),
                Money::from_minor_units(10_000)
            )
            .completion_basis_points,
            10_000
        );
    }

    #[test]
    fn uncategorized_is_derived_and_overcommitment_is_visible() {
        let account_id = AccountId::new();
        let rows = [GoalBalance {
            category_id: CategoryId::new(),
            current: Money::from_minor_units(12_000),
            target: Money::from_minor_units(15_000),
        }];
        let summary = account_summary(account_id, Money::from_minor_units(10_000), rows).unwrap();
        assert_eq!(
            summary.uncategorized_balance,
            Money::from_minor_units(-2_000)
        );
        assert_eq!(summary.overcommitted_by, Money::from_minor_units(2_000));
    }

    #[test]
    fn non_positive_goal_balances_do_not_reserve_account_cash() {
        let summary = account_summary(
            AccountId::new(),
            Money::from_minor_units(10_000),
            [GoalBalance {
                category_id: CategoryId::new(),
                current: Money::from_minor_units(-500),
                target: Money::from_minor_units(1_000),
            }],
        )
        .unwrap();
        assert!(summary.goals.is_empty());
        assert_eq!(
            summary.uncategorized_balance,
            Money::from_minor_units(10_000)
        );
    }
}
