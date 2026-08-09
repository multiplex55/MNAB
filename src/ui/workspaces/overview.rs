use crate::app::state::AppState;

pub fn show(ui: &mut egui::Ui, state: &AppState) {
    ui.heading("Overview");
    ui.label(format!("{} at a glance", state.budget_name));
    ui.label("Use Budget to plan a month, or choose an account to review transactions.");
}
