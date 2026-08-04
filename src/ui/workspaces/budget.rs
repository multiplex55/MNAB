use crate::{
    app::{dispatcher::ActionCollector, state::AppState},
    ui::budget_view,
};

/// Workspace shell for the budget grid. The immutable `BudgetMonthView` is loaded by the
/// storage/runtime layer; this view keeps only editor/navigation intent locally and never
/// recalculates budgeting rules in UI code.
pub fn show(ui: &mut egui::Ui, state: &AppState, commands: &mut ActionCollector) {
    ui.horizontal(|ui| {
        if ui.button("◀").on_hover_text("Previous month").clicked() {
            commands.push(crate::app::command::AppCommand::PreviousMonth);
        }
        ui.heading(format!(
            "Budget · {}-{:02}",
            state.selected_month.year(),
            state.selected_month.month()
        ));
        if ui.button("▶").on_hover_text("Next month").clicked() {
            commands.push(crate::app::command::AppCommand::NextMonth);
        }
    });

    if state.active_budget.is_none() {
        budget_view::empty(ui);
        return;
    }

    ui.horizontal(|ui| {
        ui.label("Ready to Assign, assigned, activity, available, targets, overspending, and card payment availability are loaded as immutable projections from QueryStore::budget_month.");
        if ui.button("Assign all RTA").clicked() {
            commands.push(crate::app::command::AppCommand::Commit);
        }
    });
    egui::ScrollArea::both()
        .id_salt("budget-virtual-grid")
        .show(ui, |ui| {
            egui::Grid::new("budget-loading-grid")
                .striped(true)
                .show(ui, |ui| {
                    for title in ["Category", "Assigned", "Activity", "Available", "Funding"] {
                        ui.strong(title);
                    }
                    ui.end_row();
                    ui.label("Budget projection requested…");
                    ui.label("—");
                    ui.label("—");
                    ui.label("—");
                    ui.label("Use keyboard navigation and dialogs after rows load.");
                    ui.end_row();
                });
        });
}
