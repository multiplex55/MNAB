use crate::domain::{AccountType, Money};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountHeader {
    pub name: String,
    pub working: String,
    pub cleared: String,
    pub uncleared: String,
    pub actions_available: bool,
    pub reconciliation_warning: bool,
}

/// Derives the complete account-header presentation model without changing ledger signs.
#[must_use]
pub fn model(
    name: &str,
    kind: AccountType,
    working: Money,
    cleared: Money,
    closed: bool,
    unreconciled: bool,
) -> AccountHeader {
    let debt = matches!(
        kind,
        AccountType::CreditCard | AccountType::Loan | AccountType::Liability
    );
    let display = |value: Money| {
        let formatted = crate::ui::format::money(value);
        if debt && value < Money::ZERO {
            // Use wide arithmetic so even the minimum ledger value remains truthful.
            format!("{} owed", formatted.trim_start_matches('-'))
        } else {
            formatted
        }
    };
    let uncleared = working.checked_sub(cleared).unwrap_or(Money::ZERO);
    AccountHeader {
        name: name.into(),
        working: display(working),
        cleared: display(cleared),
        uncleared: display(uncleared),
        actions_available: !closed,
        reconciliation_warning: unreconciled && !closed,
    }
}

pub fn show(
    ui: &mut egui::Ui,
    header: &AccountHeader,
    context: crate::app::command::CommandAvailabilityContext,
    actions: &mut crate::app::dispatcher::ActionCollector,
) {
    ui.heading(egui::RichText::new(&header.name).size(24.0).strong());
    ui.horizontal(|ui| {
        for (label, value) in [
            ("Working", &header.working),
            ("Cleared", &header.cleared),
            ("Uncleared", &header.uncleared),
        ] {
            ui.vertical(|ui| {
                ui.small(label);
                ui.strong(value);
            });
        }
    });
    ui.horizontal(|ui| {
        for (label, command) in [
            (
                "Add Transaction",
                crate::app::command::AppCommand::AddTransaction,
            ),
            ("Import", crate::app::command::AppCommand::Import),
            (
                "Reconcile",
                crate::app::command::AppCommand::ReconcileAccount,
            ),
        ] {
            ui.add_enabled_ui(header.actions_available, |ui| {
                crate::ui::widgets::action_button(ui, label, command, context, actions);
            });
        }
    });
    if header.reconciliation_warning {
        ui.label(egui::RichText::new("⚠ Reconciliation needs attention").strong())
            .on_hover_text("This account has unreconciled activity.");
    }
    if !header.actions_available {
        ui.small("Closed account · transaction actions unavailable");
    }
    ui.separator();
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn derives_balances_and_debt_display() {
        let h = model(
            "Card",
            AccountType::CreditCard,
            Money::from_minor_units(-12345),
            Money::from_minor_units(-10000),
            false,
            false,
        );
        assert_eq!(
            (h.working.as_str(), h.cleared.as_str(), h.uncleared.as_str()),
            ("$123.45 owed", "$100.00 owed", "$23.45 owed")
        );
    }
    #[test]
    fn open_accounts_offer_actions_and_surface_warning() {
        let h = model(
            "Checking",
            AccountType::Checking,
            Money::ZERO,
            Money::ZERO,
            false,
            true,
        );
        assert!(h.actions_available);
        assert!(h.reconciliation_warning);
    }
    #[test]
    fn closed_accounts_disable_actions_and_suppress_warning() {
        let h = model(
            "Old",
            AccountType::Checking,
            Money::ZERO,
            Money::ZERO,
            true,
            true,
        );
        assert!(!h.actions_available);
        assert!(!h.reconciliation_warning);
    }
}
