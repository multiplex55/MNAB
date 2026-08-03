//! Pure budgeting calculations.

pub mod balances;
pub mod budget_month;

use crate::domain::Money;

#[must_use]
pub fn available(assigned: Money, activity: Money) -> Result<Money, crate::domain::MoneyError> {
    assigned.checked_add(activity)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn money_calculation_uses_minor_units() {
        assert_eq!(
            available(
                Money::from_minor_units(1_000),
                Money::from_minor_units(-125)
            ),
            Ok(Money::from_minor_units(875))
        );
    }
}
