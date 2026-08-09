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
    pub group_id: Option<crate::domain::AccountGroupId>,
    pub note: String,
    pub favorite: bool,
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

pub fn show(ui: &mut egui::Ui, state: &mut AppState, commands: &mut ActionCollector) {
    ui.heading("All Transactions");
    ui.small("Filter by account/group, date, payee, category, amount, approval, clearance, import source, transfer, uncategorized, or reconciliation state.");
    crate::ui::workspaces::register::show_register_header(
        ui,
        crate::ui::workspaces::register::ALL_TRANSACTION_COLUMNS,
    );
    crate::ui::workspaces::register::load_state(
        ui,
        state,
        "No transactions yet",
        "Add a transaction or import a statement to get started.",
        commands,
    );
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AllTransactionsFilter {
    pub account: Option<AccountId>,
    pub account_group: Option<crate::domain::AccountGroupId>,
    pub date_from: Option<TransactionDate>,
    pub date_to: Option<TransactionDate>,
    pub payee: Option<String>,
    pub category: Option<crate::domain::CategoryId>,
    pub amount_min: Option<Money>,
    pub amount_max: Option<Money>,
    pub approval: Option<crate::domain::Approval>,
    pub clearance: Option<crate::domain::Clearance>,
    pub import_source: Option<String>,
    pub transfer_only: bool,
    pub uncategorized: bool,
    pub reconciled: Option<bool>,
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
            ..Default::default()
        };
        assert!(form.validate().is_err());
        form.opening_magnitude = "$1,234.00".into();
        assert_eq!(form.validate().unwrap().2, Money::from_minor_units(123400));
    }
}
