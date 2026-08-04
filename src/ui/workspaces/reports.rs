use crate::app::{dispatcher::ActionCollector, state::AppState};

pub fn show(ui: &mut egui::Ui, _state: &AppState, _commands: &mut ActionCollector) {
    ui.heading("Reports");
    ui.label("Report data is loading…");
}
