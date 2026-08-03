use crate::{
    app::{
        command::AppCommand,
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
pub fn show(ui: &mut egui::Ui, state: &mut AppState, commands: &mut Vec<AppCommand>) {
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
        commands.push(AppCommand::AddAccount);
    }
}
#[allow(dead_code)]
fn _money(_: Money) {}
