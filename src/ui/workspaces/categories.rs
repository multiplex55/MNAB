use crate::app::{dispatcher::ActionCollector, state::AppState};

/// Category management is intentionally independent of the report month filter.
pub fn show(ui: &mut egui::Ui, _state: &mut AppState, _actions: &mut ActionCollector) {
    ui.heading("Categories");
    ui.label("Manage category groups, categories, and goals.");
}
