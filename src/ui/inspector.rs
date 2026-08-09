use crate::app::{
    command::AppCommand,
    dispatcher::ActionCollector,
    state::{AppState, InspectorContext, OperationStatus},
};
use crate::calculation::credit_card::CreditCardResult;
use crate::domain::{Money, Reconciliation, ReconciliationState, TransactionId};
use crate::{
    domain::{ScheduledOccurrence, TargetRecommendation, TargetStatus},
    service::assignment_service::AutoAssignPreview,
};

/// A recommendation is explanatory and deliberately has no "apply automatically" path.
pub fn show_target_recommendation(ui: &mut egui::Ui, value: &TargetRecommendation) {
    ui.heading("Target recommendation");
    egui::Grid::new("target-recommendation").show(ui, |ui| {
        for (label, amount) in [
            ("Target", value.target_amount),
            ("Funded", value.funded_amount),
            ("Remaining", value.remaining_amount),
            ("Suggested this month", value.monthly_recommendation),
        ] {
            ui.label(label);
            ui.strong(crate::ui::format::money(amount));
            ui.end_row();
        }
    });
    if let Some(due) = value.due_date {
        ui.label(format!("Due {}", crate::ui::format::date(due)));
    }
    ui.label(if value.status == TargetStatus::Funded {
        "Funded"
    } else {
        "Underfunded"
    });
    ui.label(&value.rationale);
    ui.small("Recommendations never move money. Enter or approve an assignment yourself.");
}

pub fn show_occurrences(ui: &mut egui::Ui, occurrences: &[ScheduledOccurrence]) {
    ui.heading("Upcoming scheduled items");
    for item in occurrences {
        ui.horizontal(|ui| {
            ui.strong(crate::ui::format::date(item.date));
            ui.label(crate::ui::format::money(item.amount));
            let _ = ui.button("Enter now");
            let _ = ui.button("Skip");
            let _ = ui.button("Modify");
            let _ = ui.button("Dismiss");
        });
    }
}

/// Returns true only when the user accepts the reviewed proposal.
pub fn show_auto_assign_preview(ui: &mut egui::Ui, preview: &AutoAssignPreview) -> bool {
    ui.heading("Auto-assign preview");
    for change in &preview.changes {
        ui.label(format!(
            "{}: {} → {} ({})",
            change.category_id, change.before, change.after, change.delta
        ));
    }
    ui.separator();
    ui.strong(format!("Total assignment: {}", preview.total_assignment));
    ui.label(format!("Ready to Assign after: {}", preview.remaining_rta));
    for warning in &preview.warnings {
        ui.colored_label(ui.visuals().warn_fg_color, warning);
    }
    ui.horizontal(|ui| ui.button("Apply assignments").clicked())
        .inner
}

/// Render only values produced by the domain calculator; no card budgeting rule lives in UI code.
pub fn show_credit_card(ui: &mut egui::Ui, result: &CreditCardResult) {
    let c = &result.contributions;
    egui::Grid::new("credit-card-explanation").show(ui, |ui| {
        for (label, value) in [
            ("Funded spending moved", c.funded_spending_moved),
            ("Debt created", c.debt_created),
            ("Payments made", c.payments_made),
            ("Manual assignment", c.manual_assignment),
            ("Payment available", result.payment_available),
        ] {
            ui.label(label);
            ui.strong(crate::ui::format::money(value));
            ui.end_row();
        }
    });
}

pub struct ReconciliationInspector<'a> {
    pub statement_balance: Money,
    pub cleared_balance: Money,
    pub difference: Money,
    pub eligible: &'a [TransactionId],
    pub history: &'a [Reconciliation],
}

/// Reconciliation actions are buttons as well as shortcuts, so none require a pointer.
pub fn show_reconciliation(ui: &mut egui::Ui, model: &ReconciliationInspector<'_>) {
    egui::Grid::new("reconciliation-totals").show(ui, |ui| {
        ui.label("Statement balance");
        ui.strong(crate::ui::format::money(model.statement_balance));
        ui.end_row();
        ui.label("Cleared balance");
        ui.strong(crate::ui::format::money(model.cleared_balance));
        ui.end_row();
        ui.label("Difference");
        ui.strong(crate::ui::format::money(model.difference));
        ui.end_row();
    });
    ui.label(format!("{} eligible register rows", model.eligible.len()));
    ui.horizontal(|ui| {
        let _ = ui.button("Clear selected (Space)");
        let _ = ui.button("Correct selected (Enter)");
        let _ = ui.button("Preview adjustment (A)");
        ui.add_enabled(
            model.difference == Money::ZERO,
            egui::Button::new("Complete (Cmd/Ctrl+Enter)"),
        );
    });
    ui.separator();
    ui.heading("Reconciliation history");
    for record in model.history {
        let warning = if record.state == ReconciliationState::PotentiallyInvalid {
            " ⚠ potentially invalid"
        } else {
            ""
        };
        ui.label(format!(
            "{} · {} · {} transaction(s){warning}",
            record.statement_date.0,
            record.ending_balance,
            record.included_transaction_ids.len()
        ));
    }
}
pub fn show(ui: &mut egui::Ui, state: &mut AppState, actions: &mut ActionCollector) {
    ui.horizontal(|ui| {
        ui.heading("Inspector");
        if ui.button("Collapse (Cmd/Ctrl+\\)").clicked() {
            actions.push(AppCommand::ToggleInspector);
        }
    });
    match &state.inspector_context {
        InspectorContext::AccountSummary(account) => {
            ui.label("Account summary");
            ui.label(account.map_or_else(
                || "Select an account to see balances and activity.".into(),
                |id| format!("Selected account: {id}"),
            ));
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
                    actions.push(AppCommand::CancelOperation);
                }
            }
            OperationStatus::Failed(error) => {
                ui.colored_label(ui.visuals().error_fg_color, format!("{error:?}"));
                if ui.button("Retry").clicked() {
                    actions.push(AppCommand::RetryOperation);
                }
            }
        }
    }
}
