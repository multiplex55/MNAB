use crate::{
    app::{dispatcher::ActionCollector, state::AppState},
    ui::budget_view,
};

/// The workspace renders the latest accepted immutable projection. Refresh failures keep that
/// projection visible; edits remain in `BudgetUiState` until the runtime accepts a replacement.
pub fn show(ui: &mut egui::Ui, state: &mut AppState, actions: &mut ActionCollector) {
    if state.active_budget.is_none() {
        budget_view::empty(ui);
        return;
    }
    if let Some(error) = &state.budget_month.safe_failure {
        ui.colored_label(
            ui.visuals().warn_fg_color,
            format!("Budget refresh failed: {error}"),
        );
    }
    let Some(view) = state.budget_month.last_successful.clone() else {
        ui.spinner();
        ui.label("Loading Budget month…");
        return;
    };
    budget_view::show(ui, &view, &mut state.budget_ui, actions);
}
