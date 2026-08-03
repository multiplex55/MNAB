//! Pure category-target validation and recommendation calculations.
use super::{AccountId, CategoryId, Money, MoneyError, TargetId};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use time::Date;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum TargetAssociation {
    Category(CategoryId),
    /// Credit-card payoff targets belong to the card's payment category.
    CreditCard {
        account_id: AccountId,
        payment_category_id: CategoryId,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum TargetRecurrence {
    None,
    Monthly,
    Yearly,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum TargetKind {
    BalanceAmount {
        amount: Money,
    },
    BalanceByDate {
        amount: Money,
        due: Date,
    },
    FixedMonthlySavings {
        amount: Money,
    },
    RefillToAmount {
        amount: Money,
    },
    UpcomingExpense {
        amount: Money,
        due: Date,
        recurrence: TargetRecurrence,
    },
    CreditCardPayoffByDate {
        due: Date,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Target {
    pub id: TargetId,
    pub association: TargetAssociation,
    pub kind: TargetKind,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CreditCardTargetState {
    pub card_debt: Money,
    pub payment_available: Money,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TargetStatus {
    Funded,
    Underfunded,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TargetRecommendation {
    pub target_amount: Money,
    pub funded_amount: Money,
    pub remaining_amount: Money,
    pub monthly_recommendation: Money,
    pub due_date: Option<Date>,
    pub status: TargetStatus,
    pub rationale: String,
}

#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub enum TargetError {
    #[error("target amount must be positive")]
    NonPositiveAmount,
    #[error("a dated target must not be created after its due date")]
    InvalidDueDate,
    #[error("this target type is not supported by that association")]
    UnsupportedAssociation,
    #[error("recurrence is not valid for this target type")]
    InvalidRecurrence,
    #[error("credit-card state is required for a payoff target")]
    MissingCreditCardState,
    #[error("target calculation overflowed")]
    Overflow,
}

impl Target {
    pub fn new(
        association: TargetAssociation,
        kind: TargetKind,
        created: Date,
    ) -> Result<Self, TargetError> {
        let amount = match kind {
            TargetKind::BalanceAmount { amount }
            | TargetKind::BalanceByDate { amount, .. }
            | TargetKind::FixedMonthlySavings { amount }
            | TargetKind::RefillToAmount { amount }
            | TargetKind::UpcomingExpense { amount, .. } => Some(amount),
            TargetKind::CreditCardPayoffByDate { .. } => None,
        };
        if amount.is_some_and(|v| v <= Money::ZERO) {
            return Err(TargetError::NonPositiveAmount);
        }
        let due = match kind {
            TargetKind::BalanceByDate { due, .. }
            | TargetKind::UpcomingExpense { due, .. }
            | TargetKind::CreditCardPayoffByDate { due } => Some(due),
            _ => None,
        };
        if due.is_some_and(|d| d < created) {
            return Err(TargetError::InvalidDueDate);
        }
        if matches!(kind, TargetKind::CreditCardPayoffByDate { .. })
            != matches!(association, TargetAssociation::CreditCard { .. })
        {
            return Err(TargetError::UnsupportedAssociation);
        }
        Ok(Self {
            id: TargetId::new(),
            association,
            kind,
        })
    }

    /// Calculates advice only. No assignment or account value is mutated.
    pub fn recommend(
        &self,
        today: Date,
        available: Money,
        card: Option<CreditCardTargetState>,
    ) -> Result<TargetRecommendation, TargetError> {
        let funded = nonnegative(available);
        let (target, due, fixed_monthly) = match self.kind {
            TargetKind::BalanceAmount { amount } | TargetKind::RefillToAmount { amount } => {
                (amount, None, None)
            }
            TargetKind::BalanceByDate { amount, due }
            | TargetKind::UpcomingExpense { amount, due, .. } => (amount, Some(due), None),
            TargetKind::FixedMonthlySavings { amount } => (amount, None, Some(amount)),
            TargetKind::CreditCardPayoffByDate { due } => {
                let state = card.ok_or(TargetError::MissingCreditCardState)?;
                // Debt is conventionally negative; positive values are also accepted as magnitude.
                let debt = Money::from_minor_units(
                    state
                        .card_debt
                        .minor_units()
                        .unsigned_abs()
                        .try_into()
                        .map_err(|_| TargetError::Overflow)?,
                );
                (debt, Some(due), None)
            }
        };
        let effective_funded = if matches!(self.kind, TargetKind::CreditCardPayoffByDate { .. }) {
            nonnegative(card.expect("checked above").payment_available)
        } else {
            funded
        };
        let remaining = target
            .checked_sub(effective_funded)
            .map_err(map_money)?
            .max(Money::ZERO);
        let monthly = if remaining == Money::ZERO {
            Money::ZERO
        } else if let Some(value) = fixed_monthly {
            value.min(remaining)
        } else if let Some(date) = due {
            divide_ceil(remaining, months_inclusive(today, date))?
        } else {
            remaining
        };
        let status = if remaining == Money::ZERO {
            TargetStatus::Funded
        } else {
            TargetStatus::Underfunded
        };
        Ok(TargetRecommendation {
            target_amount: target,
            funded_amount: effective_funded,
            remaining_amount: remaining,
            monthly_recommendation: monthly,
            due_date: due,
            status,
            rationale: if due.is_some() {
                format!(
                    "{} remaining across {} month(s); cents round up",
                    remaining,
                    due.map_or(1, |d| months_inclusive(today, d))
                )
            } else {
                format!("{remaining} remaining toward the target")
            },
        })
    }
}

fn nonnegative(v: Money) -> Money {
    v.max(Money::ZERO)
}
fn map_money(_: MoneyError) -> TargetError {
    TargetError::Overflow
}
fn divide_ceil(value: Money, divisor: i64) -> Result<Money, TargetError> {
    let cents = value.minor_units();
    let adjusted = cents
        .checked_add(divisor - 1)
        .ok_or(TargetError::Overflow)?;
    Ok(Money::from_minor_units(adjusted / divisor))
}
/// Current and due calendar months are included. An overdue target is due now (one month).
fn months_inclusive(from: Date, due: Date) -> i64 {
    if due <= from {
        return 1;
    }
    let a = i64::from(from.year()) * 12 + i64::from(u8::from(from.month()));
    let b = i64::from(due.year()) * 12 + i64::from(u8::from(due.month()));
    (b - a + 1).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::date;
    fn m(v: i64) -> Money {
        Money::from_minor_units(v)
    }
    #[test]
    fn dated_rounding_and_states() {
        let target = Target::new(
            TargetAssociation::Category(CategoryId::new()),
            TargetKind::BalanceByDate {
                amount: m(1000),
                due: date!(2026 - 03 - 31),
            },
            date!(2026 - 01 - 01),
        )
        .unwrap();
        let before = target.recommend(date!(2026 - 01 - 15), m(0), None).unwrap();
        assert_eq!(before.monthly_recommendation, m(334));
        assert_eq!(
            target
                .recommend(date!(2026 - 03 - 31), m(900), None)
                .unwrap()
                .monthly_recommendation,
            m(100)
        );
        assert_eq!(
            target
                .recommend(date!(2026 - 04 - 01), m(1200), None)
                .unwrap()
                .status,
            TargetStatus::Funded
        );
    }
    #[test]
    fn payoff_uses_debt_and_payment_category() {
        let target = Target::new(
            TargetAssociation::CreditCard {
                account_id: AccountId::new(),
                payment_category_id: CategoryId::new(),
            },
            TargetKind::CreditCardPayoffByDate {
                due: date!(2026 - 02 - 28),
            },
            date!(2026 - 01 - 01),
        )
        .unwrap();
        let value = target
            .recommend(
                date!(2026 - 01 - 02),
                m(99999),
                Some(CreditCardTargetState {
                    card_debt: m(-1000),
                    payment_available: m(200),
                }),
            )
            .unwrap();
        assert_eq!(
            (
                value.target_amount,
                value.funded_amount,
                value.remaining_amount,
                value.monthly_recommendation
            ),
            (m(1000), m(200), m(800), m(400))
        );
    }
    #[test]
    fn validates_amount_date_and_association() {
        assert_eq!(
            Target::new(
                TargetAssociation::Category(CategoryId::new()),
                TargetKind::BalanceAmount { amount: m(0) },
                date!(2026 - 01 - 01)
            ),
            Err(TargetError::NonPositiveAmount)
        );
        assert_eq!(
            Target::new(
                TargetAssociation::Category(CategoryId::new()),
                TargetKind::BalanceByDate {
                    amount: m(1),
                    due: date!(2025 - 12 - 31)
                },
                date!(2026 - 01 - 01)
            ),
            Err(TargetError::InvalidDueDate)
        );
    }
}
