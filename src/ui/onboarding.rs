use crate::domain::{AccountType, Money, TransactionDate};
use std::collections::BTreeSet;

pub const STARTER_CATEGORIES: &[&str] =
    &["Rent/Mortgage", "Utilities", "Groceries", "Transportation"];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FirstAccountForm {
    pub name: String,
    pub account_type: AccountType,
    /// Dollars entered by the user. Debt types describe this as a positive amount owed.
    pub current_balance: String,
    pub balance_date: String,
    pub group: String,
    pub note: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OnboardingWizard {
    pub step: u8,
    pub budget_name: String,
    pub account: FirstAccountForm,
    pub selected_categories: BTreeSet<String>,
}

impl Default for OnboardingWizard {
    fn default() -> Self {
        Self {
            step: 1,
            budget_name: String::new(),
            account: FirstAccountForm {
                name: String::new(),
                account_type: AccountType::Checking,
                current_balance: "0".into(),
                balance_date: String::new(),
                group: "Cash Accounts".into(),
                note: String::new(),
            },
            selected_categories: STARTER_CATEGORIES.iter().map(|s| (*s).to_owned()).collect(),
        }
    }
}

impl OnboardingWizard {
    /// Parses exactly what the user entered. Ledger sign is a domain concern and is deliberately
    /// not applied by the form.
    pub fn parsed_opening_magnitude(&self) -> Result<Money, &'static str> {
        let value = crate::ui::budget_view::parse_usd_input(&self.account.current_balance)
            .map_err(|_| "Enter an amount such as 1234.56 or $1,234.56")?;
        if value < Money::ZERO {
            return Err("Enter a positive amount owed for debt accounts");
        }
        Ok(value)
    }
    /// The signed effect shown on the review page. This must never be used as service input.
    pub fn signed_opening_preview(&self) -> Result<Money, &'static str> {
        self.account
            .account_type
            .opening_amount(self.parsed_opening_magnitude()?)
            .map_err(|_| "Amount is too large")
    }
    pub fn parsed_date(&self) -> Result<TransactionDate, &'static str> {
        time::Date::parse(
            self.account.balance_date.trim(),
            &time::format_description::well_known::Iso8601::DATE,
        )
        .map(TransactionDate)
        .map_err(|_| "Balance date must be YYYY-MM-DD")
    }
    #[must_use]
    pub fn debt_help(&self) -> Option<&'static str> {
        matches!(
            self.account.account_type,
            AccountType::CreditCard | AccountType::Loan | AccountType::Liability
        )
        .then_some(
            "Enter the positive amount you owe. MNAB records it as a negative ledger balance.",
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn currency_and_debt_conversion() {
        let mut w = OnboardingWizard::default();
        w.account.account_type = AccountType::Loan;
        w.account.current_balance = "$1,234.56".into();
        assert_eq!(
            w.parsed_opening_magnitude().unwrap(),
            Money::from_minor_units(123456)
        );
        assert_eq!(
            w.signed_opening_preview().unwrap(),
            Money::from_minor_units(-123456)
        );
        w.account.current_balance = "0".into();
        assert_eq!(w.parsed_opening_magnitude().unwrap(), Money::ZERO);
    }
    #[test]
    fn starter_selection_can_be_empty() {
        let mut w = OnboardingWizard::default();
        w.selected_categories.clear();
        assert!(w.selected_categories.is_empty());
    }
}
