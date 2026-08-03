//! Pure budgeting calculations.

use crate::domain::Money;

#[must_use]
pub fn available(assigned: Money, activity: Money) -> Money {
    Money(assigned.0 + activity.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn money_calculation_uses_minor_units() {
        assert_eq!(available(Money(1_000), Money(-125)), Money(875));
    }
}
