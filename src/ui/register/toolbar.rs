use crate::app::{dispatcher::ActionCollector, state::AppState};

pub fn show(ui: &mut egui::Ui, state: &mut AppState, commands: &mut ActionCollector, all: bool) {
    ui.horizontal(|ui| {
        if contextual_command_visible(all) && ui.button("Add Transaction").clicked() {
            commands.push(crate::app::command::AppCommand::AddTransaction);
        }
        if contextual_command_visible(all) && ui.button("Transfer").clicked() {
            commands.push(crate::app::command::AppCommand::CreateTransfer);
        }
        if contextual_command_visible(all) && ui.button("Import").clicked() {
            commands.push(crate::app::command::AppCommand::Import);
        }
        if contextual_command_visible(all) && ui.button("Reconcile").clicked() {
            commands.push(crate::app::command::AppCommand::ReconcileAccount);
        }
        ui.separator();
        let search =
            ui.add(egui::TextEdit::singleline(&mut state.search).hint_text("Search transactions"));
        if search.changed() {
            commands.push(crate::app::command::AppCommand::FocusSearch);
        }
    });
}

/// Account-changing commands belong to a specific account register. The aggregate
/// register remains useful for searching, but intentionally has no mutation toolbar.
#[must_use]
pub const fn contextual_command_visible(all_transactions: bool) -> bool {
    !all_transactions
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_commands_are_visible_only_in_account_registers() {
        assert!(!contextual_command_visible(true));
        assert!(contextual_command_visible(false));
    }
}
