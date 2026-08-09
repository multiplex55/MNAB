use crate::app::{
    command::AppCommand,
    dispatcher::ActionCollector,
    state::{AppState, InspectorContext, OperationStatus},
};
use crate::calculation::credit_card::CreditCardResult;
use crate::domain::{Money, Reconciliation, ReconciliationState, TransactionId};
use crate::{
    domain::{
        AccountId, BudgetAssignment, ScheduledOccurrence, TargetRecommendation, TargetStatus,
    },
    service::assignment_service::AutoAssignPreview,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ResolvedInspector {
    Empty(&'static str),
    Account {
        name: String,
        working: i64,
        cleared: i64,
        tracking: bool,
        closed: bool,
        unreconciled: bool,
    },
    Transaction(crate::app::view_model::RegisterRowView),
    Category(crate::app::view_model::CategoryRowView),
}

/// Resolves stable identities exclusively through the last immutable snapshots. During refresh
/// those snapshots remain available, so the inspector does not flicker or fall back to UUID text.
#[must_use]
pub fn resolve(state: &AppState) -> ResolvedInspector {
    match state.inspector_context {
        InspectorContext::AccountSummary(Some(id)) => state
            .accounts
            .iter()
            .find(|a| a.id == id)
            .map(|a| ResolvedInspector::Account {
                name: a.name.clone(),
                working: a.working_balance.minor_units(),
                cleared: a.cleared_balance.minor_units(),
                tracking: a.tracking,
                closed: a.closed,
                unreconciled: a.unreconciled,
            })
            .unwrap_or(ResolvedInspector::Empty(
                "The selected account is no longer available.",
            )),
        InspectorContext::Transaction(Some(id)) => state
            .register_query
            .last_successful
            .as_ref()
            .and_then(|page| page.rows.iter().find(|row| row.transaction_id == id))
            .cloned()
            .map(ResolvedInspector::Transaction)
            .unwrap_or(ResolvedInspector::Empty(
                "The selected transaction is no longer available.",
            )),
        InspectorContext::BudgetCategory(Some(id)) => state
            .budget_month
            .last_successful
            .as_ref()
            .and_then(|month| month.rows.iter().find(|row| row.category_id == id))
            .cloned()
            .map(ResolvedInspector::Category)
            .unwrap_or(ResolvedInspector::Empty(
                "The selected category is no longer available.",
            )),
        InspectorContext::AccountSummary(None) => {
            ResolvedInspector::Empty("Select an account to see balances and activity.")
        }
        InspectorContext::Transaction(None) => {
            ResolvedInspector::Empty("Select a transaction to see its details.")
        }
        InspectorContext::BudgetCategory(None) => {
            ResolvedInspector::Empty("Select a budget category to see its plan.")
        }
    }
}

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
            "Category assignment: {} → {} ({})",
            change.before, change.after, change.delta
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
    match resolve(state) {
        ResolvedInspector::Empty(message) => {
            ui.label(message);
        }
        ResolvedInspector::Account {
            name,
            working,
            cleared,
            tracking,
            closed,
            unreconciled,
        } => {
            ui.heading(name);
            money_grid(
                ui,
                &[
                    ("Working", working),
                    ("Cleared", cleared),
                    ("Uncleared", working - cleared),
                ],
            );
            ui.label(if unreconciled {
                "Reconciliation: needs attention"
            } else {
                "Reconciliation: up to date"
            });
            if ui
                .add_enabled(!tracking && !closed, egui::Button::new("Reconcile"))
                .clicked()
            {
                actions.push(AppCommand::ReconcileAccount);
            }
        }
        ResolvedInspector::Transaction(row) => {
            ui.heading(if row.payee_name.is_empty() {
                "No payee"
            } else {
                &row.payee_name
            });
            ui.label(if row.is_transfer {
                format!("Transfer · {}", row.account_name)
            } else if row.category_name.is_empty() {
                "Uncategorized".into()
            } else {
                row.category_name.clone()
            });
            ui.label(crate::ui::format::date(row.date));
            money_grid(ui, &[("Amount", row.inflow_cents - row.outflow_cents)]);
            ui.label(format!("Clearance: {}", row.cleared_state));
            ui.label(if row.approved {
                "Approved"
            } else {
                "Unapproved"
            });
            if let Some(memo) = row.memo.as_deref().filter(|memo| !memo.is_empty()) {
                ui.label(format!("Memo: {memo}"));
            }
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(!row.reconciled, egui::Button::new("Edit"))
                    .clicked()
                {
                    actions.push(AppCommand::EditTransaction);
                }
                if ui
                    .add_enabled(!row.reconciled, egui::Button::new("Delete"))
                    .clicked()
                {
                    actions.push(AppCommand::DeleteTransaction);
                }
                // Transfers require two-leg duplication support which the command service does not expose.
                if ui
                    .add_enabled(!row.is_transfer, egui::Button::new("Duplicate"))
                    .clicked()
                {
                    if let Some(mut draft) = crate::ui::register::editor_from_row(
                        &row,
                        crate::app::state::EditorMetadata::new(egui::Id::new(
                            "inspector-duplicate",
                        )),
                    ) {
                        draft.transaction_id = None;
                        draft.clearance = crate::domain::Clearance::Uncleared;
                        draft.approved = false;
                        draft.reconciled = false;
                        state.editor = crate::app::state::EditorState::CreatingTransaction(draft);
                    }
                }
            });
        }
        ResolvedInspector::Category(row) => {
            ui.heading(row.name);
            money_grid(
                ui,
                &[
                    ("Available", row.available_cents),
                    ("Assigned", row.assigned_cents),
                    ("Activity", row.activity_cents),
                ],
            );
            if let Some(target) = row.target_amount_cents {
                money_grid(
                    ui,
                    &[
                        ("Target", target),
                        ("Remaining", row.target_remaining_cents.unwrap_or(0)),
                    ],
                );
            }
            if let Some(due) = row.target_due_date.as_deref() {
                ui.label(format!("Due: {due}"));
            }
            if !row.target_status.is_empty() {
                ui.label(format!("Target: {}", row.target_status));
            }
            if let Some(amount) = row.target_remaining_cents.filter(|amount| *amount > 0) {
                if ui
                    .button(format!(
                        "Assign {}",
                        crate::ui::format::money(Money::from_minor_units(amount))
                    ))
                    .clicked()
                {
                    actions.push(crate::app::command::ApplicationAction::Financial(
                        crate::app::command::FinancialCommand::Assignment(
                            crate::app::command::AssignmentCommand::Set(BudgetAssignment {
                                category_id: row.category_id,
                                month: state.selected_month,
                                amount: Money::from_minor_units(row.assigned_cents + amount),
                            }),
                        ),
                    ));
                }
            }
            ui.small(
                "Suggestions are previews. Money moves only after an explicit assignment command.",
            );
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

fn money_grid(ui: &mut egui::Ui, values: &[(&str, i64)]) {
    egui::Grid::new(ui.next_auto_id()).show(ui, |ui| {
        for (label, cents) in values {
            ui.label(*label);
            ui.strong(crate::ui::format::money(Money::from_minor_units(*cents)));
            ui.end_row();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_id_resolves_to_display_data_and_stays_during_refresh() {
        let mut state = AppState::default();
        let id = AccountId::new();
        state.accounts.push(crate::app::state::AccountSummary {
            id,
            name: "Checking".into(),
            working_balance: Money::from_minor_units(1234),
            cleared_balance: Money::from_minor_units(1000),
            unreconciled: true,
            tracking: false,
            closed: false,
            group_id: None,
            favorite: false,
            account_type: crate::domain::AccountType::Checking,
        });
        state.inspector_context = InspectorContext::AccountSummary(Some(id));
        state.account_tree.refresh_active = true;
        assert!(
            matches!(resolve(&state), ResolvedInspector::Account { name, working: 1234, .. } if name == "Checking")
        );
        state.accounts.clear();
        assert!(matches!(resolve(&state), ResolvedInspector::Empty(_)));
    }
}
