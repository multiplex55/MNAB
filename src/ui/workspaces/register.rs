use crate::{
    app::{dispatcher::ActionCollector, state::AppState},
    domain::AccountId,
};

pub fn show(
    ui: &mut egui::Ui,
    _state: &AppState,
    account_id: AccountId,
    _commands: &mut ActionCollector,
) {
    if let Some(account) = _state.accounts.iter().find(|a| a.id == account_id) {
        let h = crate::ui::account_header::format(
            &account.name,
            account.account_type,
            account.working_balance,
            account.cleared_balance,
        );
        ui.heading(h.name);
        ui.horizontal(|ui| {
            ui.label(format!("Working: {}", h.working));
            ui.label(format!("Cleared: {}", h.cleared));
            ui.label(format!("Uncleared: {}", h.uncleared));
        });
        ui.horizontal(|ui| {
            for (label, command) in [
                (
                    "New Transaction",
                    crate::app::command::AppCommand::AddTransaction,
                ),
                ("Transfer", crate::app::command::AppCommand::CreateTransfer),
                ("Import", crate::app::command::AppCommand::Import),
                (
                    "Reconcile",
                    crate::app::command::AppCommand::ReconcileAccount,
                ),
            ] {
                if ui.button(label).clicked() {
                    _commands.push(command);
                }
            }
        });
    } else {
        ui.heading("Account Transactions");
    }
    load_state(
        ui,
        _state,
        "No transactions in this account",
        "Add a transaction, transfer money, or import a statement.",
        _commands,
    );
}

pub fn load_state(
    ui: &mut egui::Ui,
    state: &AppState,
    empty_title: &str,
    empty_action: &str,
    commands: &mut ActionCollector,
) {
    let query = &state.register_query;
    if query.refresh_active && query.last_successful.is_none() {
        ui.spinner();
        ui.label("Loading transactions…");
        return;
    }
    if let Some(error) = &query.safe_failure {
        ui.colored_label(
            ui.visuals().error_fg_color,
            format!("Could not load transactions: {error}"),
        );
        if ui.button("Retry").clicked() {
            commands.push(crate::app::command::AppCommand::RetryOperation);
        }
        return;
    }
    match query.last_successful {
        Some(count) if count > 0 => {
            ui.label(format!("{count} transactions"));
            if query.refresh_active {
                ui.spinner();
                ui.small("Refreshing…");
            }
        }
        _ => {
            ui.strong(empty_title);
            ui.label(empty_action);
            if ui.button("New Transaction").clicked() {
                commands.push(crate::app::command::AppCommand::AddTransaction);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::storage::worker::Generation;

    #[test]
    fn loading_error_empty_and_populated_transitions_are_distinct() {
        let mut q = crate::app::state::ViewQueryState::<usize>::default();
        let generation = Generation { budget: 1, view: 1 };
        q.begin(1, generation, None);
        assert!(q.refresh_active && q.last_successful.is_none());
        assert!(q.fail(1, generation, "offline"));
        assert_eq!(q.safe_failure.as_deref(), Some("offline"));
        q.begin(2, generation, None);
        assert!(q.accept(2, generation, 0));
        assert_eq!(q.last_successful, Some(0));
        q.begin(3, generation, None);
        assert!(q.accept(3, generation, 8));
        assert_eq!(q.last_successful, Some(8));
    }
}
