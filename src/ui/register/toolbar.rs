use crate::app::{dispatcher::ActionCollector, state::AppState};

pub fn show(ui: &mut egui::Ui, state: &mut AppState, commands: &mut ActionCollector, all: bool) {
    ui.horizontal(|ui| {
        if ui.button("Add Transaction").clicked() {
            commands.push(crate::app::command::AppCommand::AddTransaction);
        }
        if !all && ui.button("Transfer").clicked() {
            commands.push(crate::app::command::AppCommand::CreateTransfer);
        }
        if ui.button("Import").clicked() {
            commands.push(crate::app::command::AppCommand::Import);
        }
        if !all && ui.button("Reconcile").clicked() {
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
