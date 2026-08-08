use crate::app::{dispatcher::ActionCollector, state::AppState};

/// Category management is intentionally independent of the report month filter.
pub fn show(ui: &mut egui::Ui, _state: &mut AppState, _actions: &mut ActionCollector) {
    ui.heading("Categories");
    ui.label("Organize groups and categories, inspect usage, and manage continuous goals.");
    ui.horizontal_wrapped(|ui| {
        let _ = ui.button("New group");
        let _ = ui.button("New category");
        let _ = ui.button("Show archived");
    });
    ui.separator();
    ui.strong("Category details");
    ui.label("Rename, reorder, move, hide, archive, or merge the selected category.");
    ui.label("Usage and recent transactions remain available after archive or merge.");
    ui.separator();
    ui.strong("Goal");
    egui::Grid::new("category-goal-summary")
        .num_columns(2)
        .show(ui, |ui| {
            ui.label("Goal account");
            ui.weak("Select an account");
            ui.end_row();
            ui.label("Target amount");
            ui.weak("$0.00");
            ui.end_row();
            ui.label("Optional target date");
            ui.weak("No date");
            ui.end_row();
            ui.label("Current / remaining");
            ui.weak("$0.00 / $0.00 (0%)");
            ui.end_row();
        });
    ui.horizontal_wrapped(|ui| {
        let _ = ui.button("Create or edit goal");
        let _ = ui.button("Remove goal");
        let _ = ui.button("View activity");
        let _ = ui.button("Open transactions");
        let _ = ui.button("Transfer to goal account");
    });
}
