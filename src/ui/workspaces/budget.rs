use crate::{
    app::{dispatcher::ActionCollector, state::AppState},
    ui::budget_view,
};

pub fn show(ui: &mut egui::Ui, state: &AppState, _commands: &mut ActionCollector) {
    ui.heading(format!(
        "Budget · {}-{:02}",
        state.selected_month.year(),
        state.selected_month.month()
    ));
    if state.active_budget.is_some() {
        ui.label("Budget data is loading…");
    } else {
        budget_view::empty(ui);
    }
}
