use crate::app::{dispatcher::ActionCollector, state::AppState};

#[derive(Clone)]
struct ReportControls {
    kind: usize,
    from: String,
    through: String,
    scope: usize,
    categories: String,
}
impl Default for ReportControls {
    fn default() -> Self {
        Self {
            kind: 0,
            from: String::new(),
            through: String::new(),
            scope: 0,
            categories: String::new(),
        }
    }
}

pub fn show(ui: &mut egui::Ui, state: &AppState, _commands: &mut ActionCollector) {
    ui.heading("Reports");
    let id = ui.id().with("report-controls");
    let mut controls = ui
        .ctx()
        .data_mut(|data| data.get_temp::<ReportControls>(id).unwrap_or_default());
    ui.horizontal_wrapped(|ui| {
        egui::ComboBox::from_label("Report")
            .selected_text(
                [
                    "Spending",
                    "Income vs expense",
                    "Net worth",
                    "Budget progress",
                ][controls.kind],
            )
            .show_ui(ui, |ui| {
                for (index, label) in [
                    "Spending",
                    "Income vs expense",
                    "Net worth",
                    "Budget progress",
                ]
                .iter()
                .enumerate()
                {
                    ui.selectable_value(&mut controls.kind, index, *label);
                }
            });
        ui.label("From");
        ui.text_edit_singleline(&mut controls.from);
        ui.label("Through");
        ui.text_edit_singleline(&mut controls.through);
        egui::ComboBox::from_label("Accounts")
            .selected_text(
                ["On-budget and tracking", "On-budget only", "Tracking only"][controls.scope],
            )
            .show_ui(ui, |ui| {
                for (index, label) in ["On-budget and tracking", "On-budget only", "Tracking only"]
                    .iter()
                    .enumerate()
                {
                    ui.selectable_value(&mut controls.scope, index, *label);
                }
            });
    });
    ui.horizontal(|ui| {
        ui.label("Categories");
        ui.text_edit_singleline(&mut controls.categories)
            .on_hover_text("Comma-separated category names");
        if ui.button("Refresh").clicked() {
            ui.ctx().request_repaint();
        }
    });
    ui.separator();
    if state.active_budget.is_none() {
        ui.strong("No budget is open");
        ui.label("Open a budget to run a report.");
    } else if state.operations.values().any(|operation| {
        operation.label.to_lowercase().contains("report")
            && matches!(
                operation.status,
                crate::app::state::OperationStatus::Failed(_)
            )
    }) {
        ui.colored_label(
            ui.visuals().error_fg_color,
            "The report could not be refreshed.",
        );
        if let Some(last) = &state.selected_report {
            ui.label(last);
            ui.weak("The previous result has been retained.");
        }
        if ui.button("Try again").clicked() {
            ui.ctx().request_repaint();
        }
    } else if state
        .operations
        .values()
        .any(|operation| operation.label.to_lowercase().contains("report"))
    {
        ui.spinner();
        ui.label("Refreshing report…");
        if state.selected_report.is_some() {
            ui.weak("Showing the previous result while this report refreshes.");
        }
    } else if let Some(last) = &state.selected_report {
        ui.label(last);
        ui.horizontal(|ui| {
            let _ = ui.button("Export CSV");
            ui.weak("Aggregated results only");
        });
    } else {
        ui.strong("No report results");
        ui.label("Choose a date range and filters, then refresh.");
    }
    ui.ctx().data_mut(|data| data.insert_temp(id, controls));
}
