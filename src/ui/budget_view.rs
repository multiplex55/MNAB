//! Budget table and assignment-entry widgets.
use crate::{
    app::command::AppCommand,
    calculation::budget_month::{BudgetMonthResult, FundingStatus},
    domain::{BudgetMonth, CategoryId, Money},
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BudgetRow {
    pub category_id: CategoryId,
    pub group: String,
    pub category: String,
    pub result_index: usize,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BudgetViewAction {
    ReplaceAssignment {
        category_id: CategoryId,
        amount: Money,
    },
    NavigateTo(BudgetMonth),
}

pub fn show(
    ui: &mut egui::Ui,
    month: BudgetMonth,
    rows: &[BudgetRow],
    result: &BudgetMonthResult,
    commands: &mut Vec<AppCommand>,
    actions: &mut Vec<BudgetViewAction>,
) {
    ui.horizontal(|ui| {
        if ui.button("◀").on_hover_text("Previous month").clicked() {
            commands.push(AppCommand::PreviousMonth);
        }
        ui.heading(format!("{}-{:02}", month.year(), month.month()));
        if ui.button("▶").on_hover_text("Next month").clicked() {
            commands.push(AppCommand::NextMonth);
        }
    });
    ui.label(format!("Ready to Assign: {}", result.ready_to_assign));
    egui::Grid::new("budget-grid").striped(true).show(ui, |ui| {
        for title in ["Category", "Assigned", "Activity", "Available", "Funding"] {
            ui.strong(title);
        }
        ui.end_row();
        for row in rows {
            if let Some(value) = result.categories.get(row.result_index) {
                ui.label(format!("{} / {}", row.group, row.category));
                let mut text = value.assigned.minor_units().to_string();
                let response = ui.add(egui::TextEdit::singleline(&mut text).desired_width(90.0));
                if response.lost_focus()
                    && ui.input(|i| i.key_pressed(egui::Key::Enter))
                    && let Ok(cents) = text.parse::<i64>()
                {
                    actions.push(BudgetViewAction::ReplaceAssignment {
                        category_id: row.category_id,
                        amount: Money::from_minor_units(cents),
                    });
                }
                ui.label(value.activity.to_string());
                ui.label(value.available.to_string());
                ui.label(match value.funding {
                    FundingStatus::NoTarget => "—".into(),
                    FundingStatus::Funded => "Funded".into(),
                    FundingStatus::Underfunded(v) => format!("{v} needed"),
                });
                ui.end_row();
            }
        }
    });
}

/// Used by date pickers; callers commit navigation centrally with this typed action.
pub fn navigate_to(month: BudgetMonth) -> BudgetViewAction {
    BudgetViewAction::NavigateTo(month)
}
