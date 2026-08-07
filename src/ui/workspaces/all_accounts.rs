use crate::app::{dispatcher::ActionCollector, state::AppState};
use crate::domain::{AccountId, AccountType, Money, TransactionDate};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AccountAction {
    Create {
        name: String,
        account_type: AccountType,
        opening_magnitude: Money,
        opening_date: TransactionDate,
    },
    Rename {
        id: AccountId,
        name: String,
    },
    SetNotes {
        id: AccountId,
        notes: Option<String>,
    },
    Reorder {
        id: AccountId,
        before: Option<AccountId>,
    },
    SetFavorite {
        id: AccountId,
        favorite: bool,
    },
    Close {
        id: AccountId,
        resolution: crate::service::account_service::BalanceResolution,
        date: TransactionDate,
    },
    Reopen(AccountId),
    DeleteUnused(AccountId),
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AccountDialogForm {
    pub name: String,
    pub account_type: Option<AccountType>,
    /// User-entered non-negative magnitude; the service applies the account sign.
    pub opening_magnitude: String,
    pub opening_date: String,
}

impl AccountDialogForm {
    pub fn validate(&self) -> Result<(String, AccountType, Money, TransactionDate), &'static str> {
        let name = self.name.trim();
        if name.is_empty() {
            return Err("account name is required");
        }
        let account_type = self.account_type.ok_or("account type is required")?;
        let amount = crate::ui::budget_view::parse_usd_input(&self.opening_magnitude)
            .map_err(|_| "enter an amount such as 1234.56 or $1,234.56")?;
        if amount < Money::ZERO {
            return Err("opening balance must be a non-negative magnitude");
        }
        let date = time::Date::parse(
            self.opening_date.trim(),
            &time::format_description::well_known::Iso8601::DATE,
        )
        .map_err(|_| "opening date must be YYYY-MM-DD")?;
        Ok((name.into(), account_type, amount, TransactionDate(date)))
    }
}

pub fn show(ui: &mut egui::Ui, state: &AppState, commands: &mut ActionCollector) {
    ui.heading("All Accounts");
    crate::ui::workspaces::register::load_state(
        ui,
        state,
        "No transactions yet",
        "Add a transaction or import a statement to get started.",
        commands,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dialog_requires_magnitude_and_date() {
        let mut form = AccountDialogForm {
            name: "Card".into(),
            account_type: Some(AccountType::CreditCard),
            opening_magnitude: "-1".into(),
            opening_date: "2026-08-04".into(),
        };
        assert!(form.validate().is_err());
        form.opening_magnitude = "$1,234.00".into();
        assert_eq!(form.validate().unwrap().2, Money::from_minor_units(123400));
    }
}
