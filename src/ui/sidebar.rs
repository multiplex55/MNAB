use crate::{
    app::{
        command::AppCommand,
        dispatcher::ActionCollector,
        navigation::Workspace,
        state::{AccountSummary, AppState},
    },
    domain::Money,
};

fn account_row(ui: &mut egui::Ui, account: &AccountSummary, selected: bool) -> egui::Response {
    let warning = if account.unreconciled { " ⚠" } else { "" };
    let kind = if account.tracking { " · tracking" } else { "" };
    let closed = if account.closed { " · closed" } else { "" };
    ui.selectable_label(
        selected,
        format!(
            "{}{}{}{}   {}",
            account.name, warning, kind, closed, account.working_balance
        ),
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccountSection {
    OnBudget,
    Tracking,
    Closed,
}

#[must_use]
pub fn account_section(account: &AccountSummary) -> AccountSection {
    if account.closed {
        AccountSection::Closed
    } else if account.tracking {
        AccountSection::Tracking
    } else {
        AccountSection::OnBudget
    }
}
pub fn show(ui: &mut egui::Ui, state: &mut AppState, actions: &mut ActionCollector) {
    ui.heading("MNAB");
    ui.label(&state.budget_name);
    ui.small(
        state
            .database_path
            .as_ref()
            .map_or_else(|| "No database".into(), |p| p.display().to_string()),
    );
    ui.separator();
    for (label, workspace) in [
        ("Budget", Workspace::Budget),
        ("Reports", Workspace::Reports),
        ("All Accounts", Workspace::AllAccounts),
        ("Inbox", Workspace::Inbox),
    ] {
        if ui
            .selectable_label(state.navigation.workspace == workspace, label)
            .clicked()
        {
            state.navigation.workspace = workspace;
        }
    }
    ui.separator();
    for (title, predicate) in [
        ("ON BUDGET", (false, false)),
        ("TRACKING", (true, false)),
        ("CLOSED", (false, true)),
    ] {
        ui.strong(title);
        for account in state.accounts.iter().filter(|a| {
            if predicate.1 {
                a.closed
            } else {
                !a.closed && a.tracking == predicate.0
            }
        }) {
            if account_row(ui, account, state.selected_account == Some(account.id)).clicked() {
                state.selected_account = Some(account.id);
                state.navigation.workspace = Workspace::Account(account.id);
            }
        }
    }
    if ui.button("＋ Add Account").clicked() {
        actions.push(AppCommand::AddAccount);
    }
}
#[allow(dead_code)]
fn _money(_: Money) {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::AccountId;
    #[test]
    fn closed_always_appears_in_closed_section() {
        let account = AccountSummary {
            id: AccountId::new(),
            name: "Old".into(),
            working_balance: Money::ZERO,
            unreconciled: false,
            tracking: true,
            closed: true,
        };
        assert_eq!(account_section(&account), AccountSection::Closed);
    }
}
