//! Deterministic calculation of one budget month.
//!
//! All values are inputs or results; in particular Ready to Assign is never persisted.

use super::credit_card::{CreditCardError, CreditCardInput, CreditCardResult};
use crate::domain::{BudgetMonth, CategoryId, Money, MoneyError};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BudgetAccountKind {
    Cash,
    CreditCard,
    Tracking,
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
    pub categories: Vec<CategoryInput>,
    pub prior_categories: Vec<PriorCategoryResult>,
    /// Resources already present in on-budget accounts at the start of the projection.
    pub on_budget_resources: Money,
    /// Ready to Assign carried from the preceding calculated month.
    pub prior_ready_to_assign: Money,
    /// Tracking balances are facts used by reports, but never budget resources.
    pub tracking_balances: Money,
    /// On-budget inflows without a category are new money available to assign.
    pub uncategorized_inflows: Money,
    /// Categorized activity is repeated here as an auditable aggregate input.
    pub categorized_activity: Money,
    /// Categorized outflows split by payment-account behavior.
    pub cash_spending: Money,
    pub card_spending: Money,
    /// Net assignment movement for this month. Money moves therefore net to zero.
    pub assignments: Money,
    /// Positive category availability carried into this month.
    pub positive_rollover: Money,
    pub prior_cash_overspending: Money,
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
    let mut card_debt = Money::ZERO;
    for category in &input.categories {
        let prior = input
            .prior_categories
            .iter()
            .find(|p| p.id == category.id)
            .map_or(Money::ZERO, |p| positive(p.available));
        let available = add(add(prior, category.assigned)?, category.activity)?;
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
    // Activity changes the category envelope, not the resource pool: categorized spending must
    // not consume RTA after its assignment already did so. Tracking and current account balances
    // are intentionally absent from this equation.
    let resources = add(
        add(input.on_budget_resources, input.prior_ready_to_assign)?,
        input.uncategorized_inflows,
    )?;
    let mut rta = resources.checked_sub(input.assignments)?;
    rta = rta.checked_sub(input.prior_cash_overspending)?;
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

/// Calculates materialized months oldest-first so carryover and cash overspending can only flow
/// forward. Input order is deliberately ignored, preventing a later month from contaminating an
/// earlier result.
pub fn calculate_chronological(
    mut inputs: Vec<BudgetMonthInput>,
) -> Result<Vec<BudgetMonthResult>, CalculationError> {
    inputs.sort_by_key(|input| input.month);
    let mut output: Vec<BudgetMonthResult> = Vec::with_capacity(inputs.len());
    for input in &mut inputs {
        if let Some(prior) = output.last() {
            input.prior_ready_to_assign = prior.ready_to_assign;
            input.prior_categories = prior
                .categories
                .iter()
                .map(|category| PriorCategoryResult {
                    id: category.id,
                    // Cash and card overspending do not become negative category carryover.
                    available: positive(category.available),
                })
                .collect();
            input.positive_rollover = input
                .prior_categories
                .iter()
                .try_fold(Money::ZERO, |total, category| {
                    add(total, category.available)
                })?;
            input.prior_cash_overspending =
                prior
                    .categories
                    .iter()
                    .try_fold(Money::ZERO, |total, category| match category.overspending {
                        Overspending::Cash(value) => add(total, value),
                        Overspending::None | Overspending::CreditCard(_) => Ok(total),
                    })?;
        }
        output.push(calculate(input)?);
    }
    Ok(output)
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
            categories: vec![category],
            prior_categories: vec![],
            on_budget_resources: m(0),
            prior_ready_to_assign: m(0),
            tracking_balances: m(0),
            uncategorized_inflows: m(1000),
            categorized_activity: m(0),
            cash_spending: m(0),
            card_spending: m(0),
            assignments: m(0),
            positive_rollover: m(0),
            prior_cash_overspending: m(0),
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
        assert_eq!(r.categories[0].activity, m(-200));
        assert_eq!(r.categories[0].available, m(300));
        assert_eq!(r.ready_to_assign, m(1000));
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
        i.tracking_balances = m(9000);
        i.assignments = m(100);
        i.future_assignments = m(200);
        assert_eq!(calculate(&i).unwrap().ready_to_assign, m(700));
    }

    #[test]
    fn canonical_budget_facts_have_distinct_rta_semantics() {
        let id = CategoryId::new();
        let mut i = input(CategoryInput {
            id,
            assigned: m(600),
            activity: m(-250),
            hidden: false,
            archived: false,
            target: Some(m(800)),
            credit_card_activity: m(0),
        });
        i.on_budget_resources = m(400);
        i.uncategorized_inflows = m(1_000);
        i.tracking_balances = m(50_000);
        i.categorized_activity = m(-250);
        i.cash_spending = m(250);
        i.assignments = m(600);
        let result = calculate(&i).unwrap();
        assert_eq!(result.ready_to_assign, m(800));
        assert_eq!(result.categories[0].activity, m(-250));
        assert_eq!(result.categories[0].available, m(350));
        assert_eq!(
            result.categories[0].funding,
            FundingStatus::Underfunded(m(200))
        );
    }

    #[test]
    fn refund_money_move_and_card_overspending_do_not_spend_rta_twice() {
        let first = CategoryId::new();
        let second = CategoryId::new();
        let mut i = input(CategoryInput {
            id: first,
            assigned: m(100),
            activity: m(-300),
            hidden: false,
            archived: false,
            target: None,
            credit_card_activity: m(-300),
        });
        i.categories.push(CategoryInput {
            id: second,
            assigned: m(-100),
            activity: m(75),
            hidden: false,
            archived: false,
            target: None,
            credit_card_activity: m(0),
        });
        i.assignments = m(0);
        i.categorized_activity = m(-225);
        i.card_spending = m(300);
        let result = calculate(&i).unwrap();
        assert_eq!(result.ready_to_assign, m(1000));
        assert_eq!(
            result.categories[0].overspending,
            Overspending::CreditCard(m(200))
        );
        assert_eq!(result.categories[1].available, m(-25));
        assert_eq!(result.categories[1].overspending, Overspending::Cash(m(25)));
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

    #[test]
    fn chronology_carries_positive_and_charges_cash_overspending_to_next_rta() {
        let id = CategoryId::new();
        let category = |activity| CategoryInput {
            id,
            assigned: m(0),
            activity: m(activity),
            hidden: false,
            archived: false,
            target: None,
            credit_card_activity: m(0),
        };
        let january = input(category(-1_200));
        let mut february = input(category(0));
        february.uncategorized_inflows = Money::ZERO;
        february.month = BudgetMonth::new(2026, 2).unwrap();
        let result = calculate_chronological(vec![february, january]).unwrap();
        assert_eq!(result[0].categories[0].available, m(-1_200));
        assert_eq!(result[1].categories[0].available, Money::ZERO);
        assert_eq!(result[1].ready_to_assign, m(-200));
    }
}
