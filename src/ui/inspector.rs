use crate::app::{
    command::AppCommand,
    state::{AppState, InspectorContext, OperationStatus},
};
use crate::domain::{Money, Reconciliation, ReconciliationState, TransactionId};

pub struct ReconciliationInspector<'a> {
    pub statement_balance: Money,
    pub cleared_balance: Money,
    pub difference: Money,
    pub eligible: &'a [TransactionId],
    pub history: &'a [Reconciliation],
}

/// Reconciliation actions are buttons as well as shortcuts, so none require a pointer.
pub fn show_reconciliation(ui: &mut egui::Ui, model: &ReconciliationInspector<'_>) {
    egui::Grid::new("reconciliation-totals").show(ui, |ui| {
        ui.label("Statement balance");
        ui.strong(model.statement_balance.to_string());
        ui.end_row();
        ui.label("Cleared balance");
        ui.strong(model.cleared_balance.to_string());
        ui.end_row();
        ui.label("Difference");
        ui.strong(model.difference.to_string());
        ui.end_row();
    });
    ui.label(format!("{} eligible register rows", model.eligible.len()));
    ui.horizontal(|ui| {
        let _ = ui.button("Clear selected (Space)");
        let _ = ui.button("Correct selected (Enter)");
        let _ = ui.button("Preview adjustment (A)");
        ui.add_enabled(
            model.difference == Money::ZERO,
            egui::Button::new("Complete (Cmd/Ctrl+Enter)"),
        );
    });
    ui.separator();
    ui.heading("Reconciliation history");
    for record in model.history {
        let warning = if record.state == ReconciliationState::PotentiallyInvalid {
            " ⚠ potentially invalid"
        } else {
            ""
        };
        ui.label(format!(
            "{} · {} · {} transaction(s){warning}",
            record.statement_date.0,
            record.ending_balance,
            record.included_transaction_ids.len()
        ));
    }
}
pub fn show(ui: &mut egui::Ui, state: &mut AppState, commands: &mut Vec<AppCommand>) {
    ui.horizontal(|ui| {
        ui.heading("Inspector");
        if ui.button("Collapse (Cmd/Ctrl+\\)").clicked() {
            commands.push(AppCommand::ToggleInspector);
        }
    });
    match &state.inspector_context {
        InspectorContext::Budget => {
            ui.label("Assignment inspector");
            ui.label("Enter assignment amounts in cents, then press Enter to commit.");
            ui.label("Undo restores the assignment records from before the command.");
        }
        InspectorContext::Transaction(_) => {
            ui.label("Transaction details");
        }
        InspectorContext::Reconciliation(_) => {
            ui.label("Reconciliation controls");
        }
        InspectorContext::Import(_) => {
            ui.label("Import match details");
        }
        InspectorContext::Target(_) => {
            ui.label("Target details");
        }
    }
    for operation in state.operations.values() {
        ui.separator();
        ui.label(&operation.label);
        match &operation.status {
            OperationStatus::Running { progress } => {
                if let Some(p) = progress {
                    ui.add(egui::ProgressBar::new(f32::from(*p) / 100.0).show_percentage());
                } else {
                    ui.spinner();
                }
                if ui.button("Cancel operation").clicked() {
                    commands.push(AppCommand::CancelOperation);
                }
            }
            OperationStatus::Failed(error) => {
                ui.colored_label(ui.visuals().error_fg_color, format!("{error:?}"));
                if ui.button("Retry").clicked() {
                    commands.push(AppCommand::RetryOperation);
                }
            }
        }
    }
}
