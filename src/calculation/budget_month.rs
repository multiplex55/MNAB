//! Deterministic calculation of one budget month.
//!
//! All values are inputs or results; in particular Ready to Assign is never persisted.

use super::credit_card::{CreditCardError, CreditCardInput, CreditCardResult, PriorCardState};
use crate::domain::{AccountId, BudgetMonth, CategoryId, Money, MoneyError};

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
    /// Card accounts and their payment-category mapping. Activity is ordered exactly as it
    /// occurred, and each purchase/refund carries the availability that was funded at that time.
    pub credit_cards: Vec<CreditCardAccountInput>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreditCardAccountInput {
    pub account_id: AccountId,
    pub payment_category_id: CategoryId,
    pub calculation: CreditCardInput,
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
    pub credit_cards: Vec<CreditCardAccountResult>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreditCardAccountResult {
    pub account_id: AccountId,
    pub payment_category_id: CategoryId,
    pub calculation: CreditCardResult,
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
    if !input.credit_cards.is_empty() {
        // The generic pass can only infer card debt from monthly aggregates. Once canonical card
        // events are present, discard that approximation in favor of the card engine.
        card_debt = Money::ZERO;
    }
    let mut credit_cards = Vec::with_capacity(input.credit_cards.len());
    for card in &input.credit_cards {
        let calculated = super::credit_card::calculate(&card.calculation)?;
        rta = add(rta, calculated.contributions.ready_to_assign_change)?;
        card_debt = add(card_debt, calculated.contributions.debt_created)?;

        // Card activity is already included in each ordinary category's activity. Replace only
        // its overspending classification; the card engine is authoritative for the funded split.
        for spending in &calculated.categories {
            if let Some(category) = results.iter_mut().find(|c| c.id == spending.category_id)
                && spending.card_overspending > Money::ZERO
            {
                category.overspending = Overspending::CreditCard(spending.card_overspending);
            }
        }
        // The payment envelope is special: funded purchases move cash into it and payments
        // consume it, but neither operation changes the resource pool (RTA).
        if let Some(payment) = results
            .iter_mut()
            .find(|c| c.id == card.payment_category_id)
        {
            payment.activity = calculated
                .contributions
                .funded_spending_moved
                .checked_sub(calculated.contributions.funded_refunds_reversed)?
                .checked_sub(calculated.contributions.payment_cash_used)?;
            payment.available = calculated.payment_available;
            payment.overspending = Overspending::None;
        }
        credit_cards.push(CreditCardAccountResult {
            account_id: card.account_id,
            payment_category_id: card.payment_category_id,
            calculation: calculated,
        });
    }
    Ok(BudgetMonthResult {
        month: input.month,
        categories: results,
        ready_to_assign: rta,
        credit_card_debt_created: card_debt,
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
            for card in &mut input.credit_cards {
                for spending in &mut card.calculation.categories {
                    let prior_available = input
                        .prior_categories
                        .iter()
                        .find(|category| category.id == spending.category_id)
                        .map_or(Money::ZERO, |category| positive(category.available));
                    let assigned = input
                        .categories
                        .iter()
                        .find(|category| category.id == spending.category_id)
                        .map_or(Money::ZERO, |category| category.assigned);
                    spending.available = add(prior_available, assigned)?;
                }
                if let Some(previous) = prior
                    .credit_cards
                    .iter()
                    .find(|p| p.account_id == card.account_id)
                {
                    card.calculation.prior = PriorCardState {
                        card_balance: previous.calculation.card_balance,
                        payment_available: previous.calculation.payment_available,
                        refundable_funded: previous.calculation.refundable_funded,
                    };
                }
            }
        }
        output.push(calculate(input)?);
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::calculation::credit_card::{CardActivity, SpendingCategoryInput};
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
            credit_cards: vec![],
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

    fn card_month(available: i64, amount: i64, funded: i64) -> BudgetMonthInput {
        let spending = CategoryId::new();
        let payment = CategoryId::new();
        let mut month = input(CategoryInput {
            id: spending,
            assigned: m(available),
            activity: m(-amount),
            hidden: false,
            archived: false,
            target: None,
            credit_card_activity: m(-amount),
        });
        month.categories.push(CategoryInput {
            id: payment,
            assigned: Money::ZERO,
            activity: Money::ZERO,
            hidden: false,
            archived: false,
            target: None,
            credit_card_activity: Money::ZERO,
        });
        month.assignments = m(available);
        month.credit_cards.push(CreditCardAccountInput {
            account_id: AccountId::new(),
            payment_category_id: payment,
            calculation: CreditCardInput {
                prior: PriorCardState {
                    card_balance: Money::ZERO,
                    payment_available: Money::ZERO,
                    refundable_funded: Money::ZERO,
                },
                categories: vec![SpendingCategoryInput {
                    category_id: spending,
                    available: m(available),
                }],
                payment_assignment: Money::ZERO,
                activity: vec![CardActivity::Purchase {
                    category_id: spending,
                    amount: m(amount),
                    funded: m(funded),
                }],
            },
        });
        month
    }

    #[test]
    fn canonical_card_path_preserves_rta_and_types_funding_invariants() {
        for (available, funded, debt) in [(100, 60, 0), (25, 25, 35), (0, 0, 60)] {
            let result = calculate(&card_month(available, 60, funded)).unwrap();
            assert_eq!(result.ready_to_assign, m(1000 - available));
            assert_eq!(result.categories[1].available, m(funded));
            assert_eq!(result.credit_card_debt_created, m(debt));
            assert_eq!(
                result.categories[0].overspending,
                if debt == 0 {
                    Overspending::None
                } else {
                    Overspending::CreditCard(m(debt))
                }
            );
        }
    }

    #[test]
    fn refund_and_payment_reverse_only_card_cash_and_never_rta() {
        let mut month = card_month(100, 60, 60);
        let card = &mut month.credit_cards[0].calculation;
        card.activity.extend([
            CardActivity::Refund {
                category_id: card.categories[0].category_id,
                amount: m(80),
                funded_reversal: m(60),
            },
            CardActivity::Payment { amount: m(30) },
        ]);
        month.categories[0].activity = m(20);
        let result = calculate(&month).unwrap();
        assert_eq!(result.ready_to_assign, m(900));
        assert_eq!(
            result.credit_cards[0].calculation.payment_available,
            Money::ZERO
        );
        assert_eq!(result.credit_cards[0].calculation.card_balance, m(50));
        assert_eq!(
            result.credit_cards[0].calculation.cash_payment_outflow,
            m(30)
        );
    }
}
