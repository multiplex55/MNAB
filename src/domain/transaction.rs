use super::*;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum Clearance {
    Uncleared,
    Cleared,
    Reconciled,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum Approval {
    Unapproved,
    Approved,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Subtransaction {
    pub category_id: CategoryId,
    pub amount: Money,
    pub memo: Option<String>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum TransactionBody {
    OpeningBalance {
        category_id: Option<CategoryId>,
    },
    Categorized {
        category_id: CategoryId,
    },
    Split {
        lines: Vec<Subtransaction>,
    },
    Transfer {
        transfer_id: TransferId,
        source_account_id: AccountId,
        destination_account_id: AccountId,
        amount: Money,
        other_account_id: AccountId,
        other_amount: Money,
        category_id: Option<CategoryId>,
        category_effect_account_id: Option<AccountId>,
    },
}

impl Transaction {
    pub fn validate(&self) -> Result<(), TransactionError> {
        if let TransactionBody::Split { lines } = &self.body {
            TransactionBody::split(self.amount, lines.clone())?;
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Transaction {
    pub id: TransactionId,
    pub budget_id: BudgetId,
    pub account_id: AccountId,
    pub date: TransactionDate,
    pub payee_id: Option<PayeeId>,
    pub amount: Money,
    pub memo: Option<String>,
    pub clearance: Clearance,
    pub approval: Approval,
    pub body: TransactionBody,
    /// Archived and voided records remain auditable but do not affect balances.
    pub archived: bool,
    pub voided: bool,
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum TransactionError {
    #[error("a split requires at least two lines")]
    TooFewSplitLines,
    #[error("split total overflowed")]
    SplitOverflow,
    #[error("split total does not equal its parent")]
    SplitTotalMismatch,
    #[error("a transfer must use distinct accounts")]
    SameAccount,
    #[error("transfer accounts must belong to the same budget")]
    DifferentBudgets,
    #[error("transfer amounts must be exact opposites")]
    TransferAmounts,
    #[error("category effect account must be source or destination")]
    InvalidCategoryEffectAccount,
    #[error("only one transfer leg may affect category activity")]
    MultipleCategoryEffectLegs,
}
impl TransactionBody {
    #[must_use]
    pub const fn categorized(category_id: CategoryId) -> Self {
        Self::Categorized { category_id }
    }
    pub fn split(parent: Money, lines: Vec<Subtransaction>) -> Result<Self, TransactionError> {
        if lines.len() < 2 {
            return Err(TransactionError::TooFewSplitLines);
        }
        let total = lines
            .iter()
            .try_fold(Money::ZERO, |sum, line| sum.checked_add(line.amount))
            .map_err(|_| TransactionError::SplitOverflow)?;
        if total != parent {
            return Err(TransactionError::SplitTotalMismatch);
        }
        Ok(Self::Split { lines })
    }
    pub fn split_remaining(
        parent: Money,
        lines: &[Subtransaction],
    ) -> Result<Money, TransactionError> {
        lines
            .iter()
            .try_fold(parent, |remaining, line| remaining.checked_sub(line.amount))
            .map_err(|_| TransactionError::SplitOverflow)
    }
    pub fn distribute_remainder(
        parent: Money,
        lines: &mut [Subtransaction],
    ) -> Result<(), TransactionError> {
        if lines.is_empty() {
            return Err(TransactionError::TooFewSplitLines);
        }
        let remainder = Self::split_remaining(parent, lines)?;
        lines[lines.len() - 1].amount = lines[lines.len() - 1]
            .amount
            .checked_add(remainder)
            .map_err(|_| TransactionError::SplitOverflow)?;
        Ok(())
    }
    pub fn categorized_transfer(
        transfer_id: TransferId,
        source: &Account,
        source_amount: Money,
        destination: &Account,
        destination_amount: Money,
        category_id: Option<CategoryId>,
        category_effect_account_id: Option<AccountId>,
    ) -> Result<(Self, Self), TransactionError> {
        if let Some(effect_account) = category_effect_account_id {
            if effect_account != source.id && effect_account != destination.id {
                return Err(TransactionError::InvalidCategoryEffectAccount);
            }
        }
        let (mut left, mut right) = Self::transfer(
            transfer_id,
            source,
            source_amount,
            destination,
            destination_amount,
        )?;
        for body in [&mut left, &mut right] {
            if let Self::Transfer {
                category_id: c,
                category_effect_account_id: e,
                ..
            } = body
            {
                *c = category_id;
                *e = category_effect_account_id;
            }
        }
        Ok((left, right))
    }

    pub fn transfer(
        transfer_id: TransferId,
        source: &Account,
        source_amount: Money,
        destination: &Account,
        destination_amount: Money,
    ) -> Result<(Self, Self), TransactionError> {
        if source.id == destination.id {
            return Err(TransactionError::SameAccount);
        }
        if source.budget_id != destination.budget_id {
            return Err(TransactionError::DifferentBudgets);
        }
        if source_amount
            .checked_neg()
            .map_err(|_| TransactionError::TransferAmounts)?
            != destination_amount
        {
            return Err(TransactionError::TransferAmounts);
        }
        Ok((
            Self::Transfer {
                transfer_id,
                source_account_id: source.id,
                destination_account_id: destination.id,
                amount: source_amount,
                other_account_id: destination.id,
                other_amount: destination_amount,
                category_id: None,
                category_effect_account_id: None,
            },
            Self::Transfer {
                transfer_id,
                source_account_id: source.id,
                destination_account_id: destination.id,
                amount: destination_amount,
                other_account_id: source.id,
                other_amount: source_amount,
                category_id: None,
                category_effect_account_id: None,
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    fn account(budget_id: BudgetId) -> Account {
        Account::new(budget_id, "a", AccountType::Checking)
    }
    #[test]
    fn splits_are_consistent_and_checked() {
        let c = CategoryId::new();
        let line = |n| Subtransaction {
            category_id: c,
            amount: Money::from_minor_units(n),
            memo: Some("memo".into()),
        };
        assert!(TransactionBody::split(Money::ZERO, vec![line(1)]).is_err());
        assert!(
            TransactionBody::split(Money::from_minor_units(5), vec![line(10), line(-5)]).is_ok()
        );
        assert_eq!(
            TransactionBody::split(Money::ZERO, vec![line(i64::MAX), line(1)]),
            Err(TransactionError::SplitOverflow)
        );
    }
    #[test]
    fn transfers_enforce_cross_entity_rules_and_reverse() {
        let b = BudgetId::new();
        let a = account(b);
        let other = account(b);
        let id = TransferId::new();
        let (left, right) = TransactionBody::transfer(
            id,
            &a,
            Money::from_minor_units(-10),
            &other,
            Money::from_minor_units(10),
        )
        .unwrap();
        match (left, right) {
            (
                TransactionBody::Transfer {
                    transfer_id: a_id,
                    other_account_id: a_other,
                    other_amount: a_amount,
                    ..
                },
                TransactionBody::Transfer {
                    transfer_id: b_id,
                    other_account_id: b_other,
                    other_amount: b_amount,
                    ..
                },
            ) => {
                assert_eq!(a_id, b_id);
                assert_eq!(a_other, other.id);
                assert_eq!(b_other, a.id);
                assert_eq!(a_amount.checked_neg().unwrap(), b_amount);
            }
            _ => unreachable!(),
        }
        assert!(TransactionBody::transfer(id, &a, Money::ZERO, &a, Money::ZERO).is_err());
        let foreign = account(BudgetId::new());
        assert!(
            TransactionBody::transfer(
                id,
                &a,
                Money::from_minor_units(-1),
                &foreign,
                Money::from_minor_units(1)
            )
            .is_err()
        );
        assert!(
            TransactionBody::transfer(
                id,
                &a,
                Money::from_minor_units(-1),
                &other,
                Money::from_minor_units(2)
            )
            .is_err()
        );
    }

    proptest! {
        #[test]
        fn valid_split_sum_is_parent(left in -1_000_000_i64..1_000_000, right in -1_000_000_i64..1_000_000) {
            let category_id = CategoryId::new();
            let lines = vec![
                Subtransaction { category_id, amount: Money::from_minor_units(left), memo: None },
                Subtransaction { category_id, amount: Money::from_minor_units(right), memo: None },
            ];
            let parent = Money::from_minor_units(left + right);
        let body = TransactionBody::split(parent, lines).unwrap();
            if let TransactionBody::Split { lines } = body {
                let sum = lines.into_iter().try_fold(Money::ZERO, |sum, line| sum.checked_add(line.amount)).unwrap();
                prop_assert_eq!(sum, parent);
            }
        }
    }
}

#[cfg(test)]
mod categorized_transfer_tests {
    use super::*;
    fn account(budget_id: BudgetId, name: &str) -> Account {
        Account::new(budget_id, name, AccountType::Checking)
    }

    #[test]
    fn only_selected_transfer_leg_affects_category_activity() {
        let b = BudgetId::new();
        let source = account(b, "source");
        let destination = account(b, "destination");
        let category = CategoryId::new();
        let (left, right) = TransactionBody::categorized_transfer(
            TransferId::new(),
            &source,
            Money::from_minor_units(-50),
            &destination,
            Money::from_minor_units(50),
            Some(category),
            Some(source.id),
        )
        .unwrap();
        let effect_accounts: Vec<_> = [left, right]
            .into_iter()
            .filter_map(|body| match body {
                TransactionBody::Transfer {
                    category_id: Some(_),
                    category_effect_account_id: Some(id),
                    ..
                } => Some(id),
                _ => None,
            })
            .collect();
        assert_eq!(effect_accounts, vec![source.id, source.id]);
    }

    #[test]
    fn category_effect_account_must_be_source_or_destination() {
        let b = BudgetId::new();
        let source = account(b, "source");
        let destination = account(b, "destination");
        assert_eq!(
            TransactionBody::categorized_transfer(
                TransferId::new(),
                &source,
                Money::from_minor_units(-1),
                &destination,
                Money::from_minor_units(1),
                Some(CategoryId::new()),
                Some(AccountId::new())
            ),
            Err(TransactionError::InvalidCategoryEffectAccount)
        );
    }
}
