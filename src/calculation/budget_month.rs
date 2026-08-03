//! Deterministic calculation of one budget month.
//!
//! All values are inputs or results; in particular Ready to Assign is never persisted.

use super::credit_card::{CreditCardError, CreditCardInput, CreditCardResult};
use crate::domain::{AccountId, BudgetMonth, CategoryId, Money, MoneyError};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BudgetAccountKind {
    Cash,
    CreditCard,
    Tracking,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountInput {
    pub id: AccountId,
    pub kind: BudgetAccountKind,
    /// Balance at the end of this month (opening balances are included here).
    pub balance: Money,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CategoryInput {
    pub id: CategoryId,
    pub assigned: Money,
    /// Categorized transaction total. Spending is negative and refunds are positive.
    pub activity: Money,
    pub hidden: bool,
    pub archived: bool,
    pub target: Option<Money>,
    pub credit_card_activity: Money,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PriorCategoryResult {
    pub id: CategoryId,
    pub available: Money,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BudgetMonthInput {
    pub month: BudgetMonth,
    pub accounts: Vec<AccountInput>,
    pub categories: Vec<CategoryInput>,
    pub prior_categories: Vec<PriorCategoryResult>,
    /// Categorized and uncategorized on-budget inflows during the displayed timeline.
    pub inflows: Money,
    pub prior_cash_overspending: Money,
    pub manual_adjustments: Money,
    /// Assignments made after this month but visible in the displayed timeline.
    pub future_assignments: Money,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Overspending {
    None,
    Cash(Money),
    CreditCard(Money),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FundingStatus {
    NoTarget,
    Funded,
    Underfunded(Money),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CategoryResult {
    pub id: CategoryId,
    pub assigned: Money,
    pub activity: Money,
    pub available: Money,
    pub funding: FundingStatus,
    pub overspending: Overspending,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BudgetMonthResult {
    pub month: BudgetMonth,
    pub categories: Vec<CategoryResult>,
    pub ready_to_assign: Money,
    pub credit_card_debt_created: Money,
}

#[derive(Clone, Debug, thiserror::Error, Eq, PartialEq)]
pub enum CalculationError {
    #[error("money arithmetic overflow")]
    Overflow,
}
impl From<CreditCardError> for CalculationError {
    fn from(_: CreditCardError) -> Self {
        Self::Overflow
    }
}

/// The single integration boundary used by UI/reporting code. Card rules remain in
/// `credit_card`; this layer only combines their documented RTA effects.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BudgetWithCardsResult {
    pub budget: BudgetMonthResult,
    pub credit_cards: Vec<CreditCardResult>,
}
impl From<MoneyError> for CalculationError {
    fn from(_: MoneyError) -> Self {
        Self::Overflow
    }
}

fn add(a: Money, b: Money) -> Result<Money, CalculationError> {
    Ok(a.checked_add(b)?)
}
fn positive(value: Money) -> Money {
    if value > Money::ZERO {
        value
    } else {
        Money::ZERO
    }
}
fn negative_magnitude(value: Money) -> Result<Money, CalculationError> {
    if value < Money::ZERO {
        Ok(value.checked_neg()?)
    } else {
        Ok(Money::ZERO)
    }
}

/// Calculates a month without I/O, clocks, global state, or mutation of its input.
pub fn calculate(input: &BudgetMonthInput) -> Result<BudgetMonthResult, CalculationError> {
    let mut results = Vec::with_capacity(input.categories.len());
    let mut assigned_total = Money::ZERO;
    let mut card_debt = Money::ZERO;
    for category in &input.categories {
        let prior = input
            .prior_categories
            .iter()
            .find(|p| p.id == category.id)
            .map_or(Money::ZERO, |p| positive(p.available));
        let available = add(add(prior, category.assigned)?, category.activity)?;
        assigned_total = add(assigned_total, category.assigned)?;
        let cash_shortfall = negative_magnitude(available)?;
        let card_shortfall =
            if cash_shortfall > Money::ZERO && category.credit_card_activity < Money::ZERO {
                let card_spend = negative_magnitude(category.credit_card_activity)?;
                if card_spend < cash_shortfall {
                    card_spend
                } else {
                    cash_shortfall
                }
            } else {
                Money::ZERO
            };
        card_debt = add(card_debt, card_shortfall)?;
        let overspending = if card_shortfall > Money::ZERO {
            Overspending::CreditCard(card_shortfall)
        } else if cash_shortfall > Money::ZERO {
            Overspending::Cash(cash_shortfall)
        } else {
            Overspending::None
        };
        let funding = category.target.map_or(FundingStatus::NoTarget, |target| {
            if category.assigned >= target {
                FundingStatus::Funded
            } else {
                FundingStatus::Underfunded(
                    target.checked_sub(category.assigned).unwrap_or(Money::ZERO),
                )
            }
        });
        results.push(CategoryResult {
            id: category.id,
            assigned: category.assigned,
            activity: category.activity,
            available,
            funding,
            overspending,
        });
    }
    let cash = input
        .accounts
        .iter()
        .filter(|a| a.kind == BudgetAccountKind::Cash)
        .try_fold(Money::ZERO, |sum, a| add(sum, a.balance))?;
    // The balance model naturally includes opening balances. Inflows are supplied separately
    // for ledger-based callers; use them only when no balance snapshot is available.
    let resources = if input
        .accounts
        .iter()
        .any(|a| a.kind == BudgetAccountKind::Cash)
    {
        cash
    } else {
        input.inflows
    };
    let mut rta = resources.checked_sub(assigned_total)?;
    rta = rta.checked_sub(input.prior_cash_overspending)?;
    rta = add(rta, input.manual_adjustments)?;
    rta = rta.checked_sub(input.future_assignments)?;
    Ok(BudgetMonthResult {
        month: input.month,
        categories: results,
        ready_to_assign: rta,
        credit_card_debt_created: card_debt,
    })
}

pub fn calculate_with_credit_cards(
    input: &BudgetMonthInput,
    cards: &[CreditCardInput],
) -> Result<BudgetWithCardsResult, CalculationError> {
    let mut budget = calculate(input)?;
    let credit_cards = cards
        .iter()
        .map(super::credit_card::calculate)
        .collect::<Result<Vec<_>, _>>()?;
    for card in &credit_cards {
        budget.ready_to_assign = add(
            budget.ready_to_assign,
            card.contributions.ready_to_assign_change,
        )?;
        budget.credit_card_debt_created = add(
            budget.credit_card_debt_created,
            card.contributions.debt_created,
        )?;
    }
    Ok(BudgetWithCardsResult {
        budget,
        credit_cards,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn m(v: i64) -> Money {
        Money::from_minor_units(v)
    }
    fn input(category: CategoryInput) -> BudgetMonthInput {
        BudgetMonthInput {
            month: BudgetMonth::new(2026, 1).unwrap(),
            accounts: vec![],
            categories: vec![category],
            prior_categories: vec![],
            inflows: m(1000),
            prior_cash_overspending: m(0),
            manual_adjustments: m(0),
            future_assignments: m(0),
        }
    }
    #[test]
    fn assignment_spending_and_refund_keep_sign() {
        let id = CategoryId::new();
        let mut i = input(CategoryInput {
            id,
            assigned: m(500),
            activity: m(-200),
            hidden: false,
            archived: false,
            target: Some(m(500)),
            credit_card_activity: m(0),
        });
        let r = calculate(&i).unwrap();
        assert_eq!(r.categories[0].available, m(300));
        i.categories[0].activity = m(100);
        assert_eq!(calculate(&i).unwrap().categories[0].available, m(600));
    }
    #[test]
    fn positive_rollover_and_hidden_history() {
        let id = CategoryId::new();
        let mut i = input(CategoryInput {
            id,
            assigned: m(0),
            activity: m(-100),
            hidden: true,
            archived: true,
            target: None,
            credit_card_activity: m(0),
        });
        i.prior_categories.push(PriorCategoryResult {
            id,
            available: m(400),
        });
        assert_eq!(calculate(&i).unwrap().categories[0].available, m(300));
    }
    #[test]
    fn tracking_and_future_assignment() {
        let id = CategoryId::new();
        let mut i = input(CategoryInput {
            id,
            assigned: m(100),
            activity: m(0),
            hidden: false,
            archived: false,
            target: None,
            credit_card_activity: m(0),
        });
        i.accounts = vec![
            AccountInput {
                id: AccountId::new(),
                kind: BudgetAccountKind::Cash,
                balance: m(1000),
            },
            AccountInput {
                id: AccountId::new(),
                kind: BudgetAccountKind::Tracking,
                balance: m(9000),
            },
        ];
        i.future_assignments = m(200);
        assert_eq!(calculate(&i).unwrap().ready_to_assign, m(700));
    }
    #[test]
    fn overflow_is_typed() {
        let id = CategoryId::new();
        let i = input(CategoryInput {
            id,
            assigned: m(i64::MAX),
            activity: m(1),
            hidden: false,
            archived: false,
            target: None,
            credit_card_activity: m(0),
        });
        assert_eq!(calculate(&i), Err(CalculationError::Overflow));
    }
}
