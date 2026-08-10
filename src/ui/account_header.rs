use crate::domain::{AccountType, Money};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountHeader {
    pub name: String,
    pub working: String,
    pub cleared: String,
    pub uncleared: String,
}

/// Derives the complete account-header presentation model without changing ledger signs.
#[must_use]
pub fn model(name: &str, kind: AccountType, working: Money, cleared: Money) -> AccountHeader {
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
    }
}

pub fn show(ui: &mut egui::Ui, header: &AccountHeader) {
    ui.heading(egui::RichText::new(&header.name).size(24.0).strong());
    ui.horizontal(|ui| {
        for (label, value) in [
            ("Working Balance", &header.working),
            ("Cleared", &header.cleared),
            ("Uncleared", &header.uncleared),
        ] {
            ui.vertical(|ui| {
                ui.small(label);
                if label == "Working Balance" {
                    ui.heading(value);
                } else {
                    ui.strong(value);
                }
            });
        }
    });
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
        );
        assert_eq!(
            (h.working.as_str(), h.cleared.as_str(), h.uncleared.as_str()),
            ("$123.45 owed", "$100.00 owed", "$23.45 owed")
        );
    }
    #[test]
    fn header_model_owns_only_identity_and_balances() {
        let h = model("Checking", AccountType::Checking, Money::ZERO, Money::ZERO);
        assert_eq!(h.name, "Checking");
    }
}
