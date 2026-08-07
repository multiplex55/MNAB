use crate::domain::{AccountType, Money};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountHeader {
    pub name: String,
    pub working: String,
    pub cleared: String,
    pub uncleared: String,
}

/// Builds labels at the presentation boundary. Debt ledgers stay negative internally while the
/// person using MNAB sees a positive amount owed.
#[must_use]
pub fn format(name: &str, kind: AccountType, working: Money, cleared: Money) -> AccountHeader {
    let debt = matches!(
        kind,
        AccountType::CreditCard | AccountType::Loan | AccountType::Liability
    );
    let display = |value: Money| {
        if debt && value < Money::ZERO {
            format!("{} owed", value.checked_neg().unwrap_or(Money::ZERO))
        } else {
            value.to_string()
        }
    };
    let uncleared = working.checked_sub(cleared).unwrap_or(Money::ZERO);
    AccountHeader {
        name: name.into(),
        working: display(working),
        cleared: display(cleared),
        uncleared: display(uncleared),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn card_uses_amount_owed_without_changing_ledger_sign() {
        let h = format(
            "Card",
            AccountType::CreditCard,
            Money::from_minor_units(-12345),
            Money::from_minor_units(-10000),
        );
        assert_eq!(h.working, "$123.45 owed");
        assert_eq!(h.uncleared, "$23.45 owed");
    }
}
