use crate::app::{dispatcher::ActionCollector, state::AppState};

pub const ACCOUNT_ACTIONS: [(&str, crate::app::command::AppCommand); 4] = [
    (
        "Add Transaction",
        crate::app::command::AppCommand::AddTransaction,
    ),
    ("Transfer", crate::app::command::AppCommand::CreateTransfer),
    ("Import", crate::app::command::AppCommand::Import),
    (
        "Reconcile",
        crate::app::command::AppCommand::ReconcileAccount,
    ),
];

#[must_use]
pub fn search_hint(state: &AppState, all: bool) -> String {
    if all {
        return "Search all transactions…".into();
    }
    let name = state
        .selected_account
        .and_then(|id| state.accounts.iter().find(|a| a.id == id))
        .map_or("account", |a| a.name.as_str());
    format!("Search {name}…")
}

pub fn show(ui: &mut egui::Ui, state: &mut AppState, commands: &mut ActionCollector, all: bool) {
    let context = state.action_context();
    ui.horizontal(|ui| {
        if contextual_command_visible(all) {
            let open = state
                .selected_account
                .and_then(|id| state.accounts.iter().find(|a| a.id == id))
                .is_some_and(|account| !account.closed);
            ui.add_enabled_ui(open, |ui| {
                for (label, command) in ACCOUNT_ACTIONS {
                    crate::ui::widgets::action_button(ui, label, command, context, commands);
                }
            });
            ui.separator();
        }
        let hint = search_hint(state, all);
        let response = ui.add(
            egui::TextEdit::singleline(&mut state.search)
                .hint_text(hint)
                .desired_width(220.0)
                .id(state.search_id),
        );
        if ui
            .ctx()
            .input(|i| i.modifiers.command && i.key_pressed(egui::Key::F))
        {
            response.request_focus();
        }
        ui.menu_button("View", |ui| view_menu(ui, state, commands));
    });
}

fn view_menu(ui: &mut egui::Ui, state: &mut AppState, commands: &mut ActionCollector) {
    for (column, label) in [
        ("running_balance", "Running Balance"),
        ("memo", "Memo"),
        ("cleared", "Cleared"),
        ("approved", "Approved"),
    ] {
        let mut visible = !state.register_columns.hidden.iter().any(|v| v == column);
        if ui.checkbox(&mut visible, label).changed() {
            state.register_columns.set_visible(column, visible);
            commands.push(crate::app::command::AppCommand::PersistRegisterView);
        }
    }
    ui.separator();
    for (density, label) in [
        (crate::app::settings::DisplayDensity::Compact, "Compact"),
        (crate::app::settings::DisplayDensity::Normal, "Normal"),
        (
            crate::app::settings::DisplayDensity::Comfortable,
            "Comfortable",
        ),
    ] {
        if ui
            .radio_value(&mut state.display_density, density, label)
            .changed()
        {
            commands.push(crate::app::command::AppCommand::PersistRegisterView);
        }
    }
    ui.separator();
    if ui.button("Reset Columns").clicked() {
        state.register_columns.reset();
        commands.push(crate::app::command::AppCommand::ResetRegisterColumns);
        ui.close();
    }
}

#[must_use]
pub const fn contextual_command_visible(all_transactions: bool) -> bool {
    !all_transactions
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn actions_have_required_order() {
        assert_eq!(
            ACCOUNT_ACTIONS.map(|a| a.0),
            ["Add Transaction", "Transfer", "Import", "Reconcile"]
        );
    }
    #[test]
    fn aggregate_has_no_account_commands() {
        assert!(!contextual_command_visible(true));
    }
    #[test]
    fn aggregate_hint_is_specific() {
        assert_eq!(
            search_hint(&AppState::default(), true),
            "Search all transactions…"
        );
    }
    #[test]
    fn optional_columns_toggle_and_reset() {
        let mut c = crate::app::settings::RegisterColumns::default();
        assert!(c.set_visible("memo", false));
        assert!(c.hidden.contains(&"memo".into()));
        c.reset();
        assert_eq!(c, Default::default());
    }
}
