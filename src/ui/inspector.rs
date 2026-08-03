use crate::app::{
    command::AppCommand,
    state::{AppState, InspectorContext, OperationStatus},
};
pub fn show(ui: &mut egui::Ui, state: &mut AppState, commands: &mut Vec<AppCommand>) {
    ui.horizontal(|ui| {
        ui.heading("Inspector");
        if ui.button("Collapse (Cmd/Ctrl+\\)").clicked() {
            commands.push(AppCommand::ToggleInspector);
        }
    });
    match &state.inspector_context {
        InspectorContext::Budget => {
            ui.label("Budget details and assignment summary");
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
