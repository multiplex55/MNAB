//! Pure credit-card budgeting rules.
//!
//! Amounts in this API are positive magnitudes. Purchases (including interest and fees) make the
//! ledger balance more negative. A caller determines funding at transaction time; consequently a
//! later purchase cannot consume money assigned after it occurred. Refunds carry the funded amount
//! of the purchase they reverse (including a purchase from a prior month). Any excess is restored
//! to the spending category but never manufactures payment-category cash. Payments are transfers:
//! they may produce a positive card balance, while only payment-category cash actually available is
//! consumed. Card overspending becomes debt and does not roll forward as negative category cash.

use crate::domain::{CategoryId, Money, MoneyError};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PriorCardState {
    pub card_balance: Money,
    pub payment_available: Money,
    /// Funded purchase amounts still eligible to be reversed by later refunds.
    pub refundable_funded: Money,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SpendingCategoryInput {
    pub category_id: CategoryId,
    pub available: Money,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CardActivity {
    /// `funded` is the portion covered by category availability when this line occurred.
    Purchase {
        category_id: CategoryId,
        amount: Money,
        funded: Money,
    },
    /// `funded_reversal` is limited to funded spending associated with the returned purchase.
    Refund {
        category_id: CategoryId,
        amount: Money,
        funded_reversal: Money,
    },
    /// A payment is the card side of a paired transfer from an on-budget cash account.
    Payment { amount: Money },
    /// Cashback/statement credit categorized back to a category or directly to Ready to Assign.
    Credit {
        amount: Money,
        category_id: Option<CategoryId>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreditCardInput {
    pub prior: PriorCardState,
    pub categories: Vec<SpendingCategoryInput>,
    /// Direct assignment (or removal) in the managed payment category, including debt paydown.
    pub payment_assignment: Money,
    pub activity: Vec<CardActivity>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CardContributions {
    pub purchases: Money,
    pub refunds: Money,
    pub credits: Money,
    pub funded_spending_moved: Money,
    pub funded_refunds_reversed: Money,
    pub debt_created: Money,
    pub payments_made: Money,
    pub payment_cash_used: Money,
    pub manual_assignment: Money,
    pub ready_to_assign_change: Money,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SpendingCategoryResult {
    pub category_id: CategoryId,
    pub available: Money,
    pub card_overspending: Money,
    /// Card overspending is debt, so only positive availability rolls forward.
    pub next_month_available: Money,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreditCardResult {
    pub card_balance: Money,
    /// Exact opposite account effect of payments; subtract this from the cash account.
    pub cash_payment_outflow: Money,
    pub payment_available: Money,
    pub refundable_funded: Money,
    pub categories: Vec<SpendingCategoryResult>,
    pub contributions: CardContributions,
}

#[derive(Clone, Debug, thiserror::Error, Eq, PartialEq)]
pub enum CreditCardError {
    #[error("amounts and funded portions must be non-negative")]
    NegativeMagnitude,
    #[error("funded portion exceeds the activity amount")]
    InvalidFundedPortion,
    #[error("unknown spending category")]
    UnknownCategory,
    #[error("money arithmetic overflow")]
    Overflow,
}
impl From<MoneyError> for CreditCardError {
    fn from(_: MoneyError) -> Self {
        Self::Overflow
    }
}
fn add(a: Money, b: Money) -> Result<Money, CreditCardError> {
    Ok(a.checked_add(b)?)
}
fn sub(a: Money, b: Money) -> Result<Money, CreditCardError> {
    Ok(a.checked_sub(b)?)
}
fn min(a: Money, b: Money) -> Money {
    if a < b { a } else { b }
}
fn positive(a: Money) -> Money {
    if a > Money::ZERO { a } else { Money::ZERO }
}
fn validate(amount: Money, funded: Money) -> Result<(), CreditCardError> {
    if amount < Money::ZERO || funded < Money::ZERO {
        return Err(CreditCardError::NegativeMagnitude);
    }
    if funded > amount {
        return Err(CreditCardError::InvalidFundedPortion);
    }
    Ok(())
}

/// Calculate one card/month. Activity order is significant and should match ledger order.
pub fn calculate(input: &CreditCardInput) -> Result<CreditCardResult, CreditCardError> {
    let mut categories: BTreeMap<_, _> = input
        .categories
        .iter()
        .map(|c| (c.category_id, c.available))
        .collect();
    let mut card = input.prior.card_balance;
    let mut payment = add(input.prior.payment_available, input.payment_assignment)?;
    let mut refundable = input.prior.refundable_funded;
    let mut c = CardContributions {
        manual_assignment: input.payment_assignment,
        ..Default::default()
    };
    let mut overspending: BTreeMap<CategoryId, Money> = BTreeMap::new();
    for event in &input.activity {
        match *event {
            CardActivity::Purchase {
                category_id,
                amount,
                funded,
            } => {
                validate(amount, funded)?;
                let available = categories
                    .get_mut(&category_id)
                    .ok_or(CreditCardError::UnknownCategory)?;
                *available = sub(*available, amount)?;
                card = sub(card, amount)?;
                payment = add(payment, funded)?;
                refundable = add(refundable, funded)?;
                c.purchases = add(c.purchases, amount)?;
                c.funded_spending_moved = add(c.funded_spending_moved, funded)?;
                let debt = sub(amount, funded)?;
                c.debt_created = add(c.debt_created, debt)?;
                *overspending.entry(category_id).or_default() = add(
                    *overspending.get(&category_id).unwrap_or(&Money::ZERO),
                    debt,
                )?;
            }
            CardActivity::Refund {
                category_id,
                amount,
                funded_reversal,
            } => {
                validate(amount, funded_reversal)?;
                let reverse = min(min(funded_reversal, refundable), positive(payment));
                let available = categories
                    .get_mut(&category_id)
                    .ok_or(CreditCardError::UnknownCategory)?;
                *available = add(*available, amount)?;
                card = add(card, amount)?;
                payment = sub(payment, reverse)?;
                refundable = sub(refundable, reverse)?;
                c.refunds = add(c.refunds, amount)?;
                c.funded_refunds_reversed = add(c.funded_refunds_reversed, reverse)?;
            }
            CardActivity::Payment { amount } => {
                validate(amount, Money::ZERO)?;
                card = add(card, amount)?;
                let used = min(amount, positive(payment));
                payment = sub(payment, used)?;
                c.payments_made = add(c.payments_made, amount)?;
                c.payment_cash_used = add(c.payment_cash_used, used)?;
            }
            CardActivity::Credit {
                amount,
                category_id,
            } => {
                validate(amount, Money::ZERO)?;
                card = add(card, amount)?;
                c.credits = add(c.credits, amount)?;
                if let Some(id) = category_id {
                    let available = categories
                        .get_mut(&id)
                        .ok_or(CreditCardError::UnknownCategory)?;
                    *available = add(*available, amount)?;
                } else {
                    c.ready_to_assign_change = add(c.ready_to_assign_change, amount)?;
                }
            }
        }
    }
    let categories = categories
        .into_iter()
        .map(|(category_id, available)| SpendingCategoryResult {
            category_id,
            available,
            card_overspending: overspending
                .get(&category_id)
                .copied()
                .unwrap_or(Money::ZERO),
            next_month_available: positive(available),
        })
        .collect();
    Ok(CreditCardResult {
        card_balance: card,
        cash_payment_outflow: c.payments_made,
        payment_available: payment,
        refundable_funded: refundable,
        categories,
        contributions: c,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    fn m(v: i64) -> Money {
        Money::from_minor_units(v)
    }
    fn run(
        open: i64,
        prior_card: i64,
        prior_payment: i64,
        assignment: i64,
        activity: Vec<CardActivity>,
    ) -> CreditCardResult {
        calculate(&CreditCardInput {
            prior: PriorCardState {
                card_balance: m(prior_card),
                payment_available: m(prior_payment),
                refundable_funded: m(prior_payment),
            },
            categories: vec![SpendingCategoryInput {
                category_id: CategoryId::from_uuid(uuid::Uuid::nil()),
                available: m(open),
            }],
            payment_assignment: m(assignment),
            activity,
        })
        .unwrap()
    }
    fn id() -> CategoryId {
        CategoryId::from_uuid(uuid::Uuid::nil())
    }
    fn buy(amount: i64, funded: i64) -> CardActivity {
        CardActivity::Purchase {
            category_id: id(),
            amount: m(amount),
            funded: m(funded),
        }
    }
    fn refund(amount: i64, funded: i64) -> CardActivity {
        CardActivity::Refund {
            category_id: id(),
            amount: m(amount),
            funded_reversal: m(funded),
        }
    }
    #[test]
    fn table_driven_purchase_payment_and_rollover_scenarios() {
        // open, starting debt, payment available, assignment, events,
        // card, spending available, payment available, debt, next-month spending
        let cases = vec![
            (
                "funded purchase",
                100,
                0,
                0,
                0,
                vec![buy(60, 60)],
                -60,
                40,
                60,
                0,
                40,
            ),
            (
                "unfunded purchase",
                0,
                0,
                0,
                0,
                vec![buy(60, 0)],
                -60,
                -60,
                0,
                60,
                0,
            ),
            (
                "partial funding",
                25,
                0,
                0,
                0,
                vec![buy(60, 25)],
                -60,
                -35,
                25,
                35,
                0,
            ),
            (
                "full payment",
                100,
                0,
                0,
                0,
                vec![buy(60, 60), CardActivity::Payment { amount: m(60) }],
                0,
                40,
                0,
                0,
                40,
            ),
            (
                "partial payment",
                100,
                0,
                0,
                0,
                vec![buy(60, 60), CardActivity::Payment { amount: m(20) }],
                -40,
                40,
                40,
                0,
                40,
            ),
            (
                "same month refund",
                100,
                0,
                0,
                0,
                vec![buy(60, 60), refund(60, 60)],
                0,
                100,
                0,
                0,
                100,
            ),
            (
                "later month refund",
                0,
                -60,
                60,
                0,
                vec![refund(60, 60)],
                0,
                60,
                0,
                0,
                60,
            ),
            (
                "starting debt no assignment",
                0,
                -100,
                0,
                0,
                vec![],
                -100,
                0,
                0,
                0,
                0,
            ),
            (
                "starting debt assignment",
                0,
                -100,
                0,
                40,
                vec![],
                -100,
                0,
                40,
                0,
                0,
            ),
            (
                "positive balance overpayment",
                0,
                -20,
                30,
                0,
                vec![CardActivity::Payment { amount: m(30) }],
                10,
                0,
                0,
                0,
                0,
            ),
            (
                "interest or fee",
                50,
                -100,
                0,
                0,
                vec![buy(20, 20)],
                -120,
                30,
                20,
                0,
                30,
            ),
            (
                "split funded/unfunded",
                30,
                0,
                0,
                0,
                vec![buy(20, 20), buy(25, 10)],
                -45,
                -15,
                30,
                15,
                0,
            ),
        ];
        for (
            name,
            open,
            card,
            pay,
            assign,
            events,
            want_card,
            want_spend,
            want_pay,
            want_debt,
            want_next,
        ) in cases
        {
            let r = run(open, card, pay, assign, events);
            assert_eq!(r.card_balance, m(want_card), "{name}: card");
            assert_eq!(r.categories[0].available, m(want_spend), "{name}: spending");
            assert_eq!(r.payment_available, m(want_pay), "{name}: payment");
            assert_eq!(r.contributions.debt_created, m(want_debt), "{name}: debt");
            assert_eq!(
                r.categories[0].next_month_available,
                m(want_next),
                "{name}: rollover"
            );
            assert_eq!(
                r.contributions.ready_to_assign_change,
                Money::ZERO,
                "{name}: RTA"
            );
        }
    }
    #[test]
    fn credits_and_excess_returns_have_explicit_disposition() {
        let r = run(
            0,
            -10,
            0,
            0,
            vec![
                refund(20, 10),
                CardActivity::Credit {
                    amount: m(5),
                    category_id: None,
                },
            ],
        );
        assert_eq!(r.card_balance, m(15));
        assert_eq!(r.categories[0].available, m(20));
        assert_eq!(r.payment_available, Money::ZERO);
        assert_eq!(r.contributions.ready_to_assign_change, m(5));
    }
    proptest! {
        #[test]
        fn funded_move_never_exceeds_eligible(amount in 0_i64..1_000_000, funded in 0_i64..1_000_000) {
            let funded = funded.min(amount);
            let r = run(amount, 0, 0, 0, vec![buy(amount, funded)]);
            prop_assert!(r.contributions.funded_spending_moved <= m(amount));
        }
        #[test]
        fn payment_transfer_is_conserved(debt in 0_i64..1_000_000, payment in 0_i64..1_000_000) {
            let r = run(0, -debt, payment, 0, vec![CardActivity::Payment { amount:m(payment) }]);
            prop_assert_eq!(r.card_balance, m(-debt + payment));
            prop_assert_eq!(r.cash_payment_outflow, m(payment));
        }
    }
}
