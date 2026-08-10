use crate::app::state::{AccountSummary, ReconciliationEditorState};

pub fn difference(
    editor: &ReconciliationEditorState,
    accounts: &[AccountSummary],
) -> Option<crate::domain::Money> {
    let cleared = editor
        .account_id
        .and_then(|id| accounts.iter().find(|a| a.id == id))
        .map_or(crate::domain::Money::ZERO, |a| a.cleared_balance);
    let statement = editor
        .statement_balance
        .parse::<crate::domain::Money>()
        .ok()?;
    crate::domain::reconciliation_difference(statement, cleared).ok()
}

pub fn show(
    ui: &mut egui::Ui,
    editor: &mut ReconciliationEditorState,
    accounts: &[AccountSummary],
) {
    let cleared = editor
        .account_id
        .and_then(|id| accounts.iter().find(|a| a.id == id))
        .map_or(crate::domain::Money::ZERO, |a| a.cleared_balance);
    ui.label(format!("Current cleared balance: {cleared}"));
    ui.label("Bank/statement cleared balance");
    editor.metadata.dirty |= ui
        .text_edit_singleline(&mut editor.statement_balance)
        .changed();
    ui.label("Statement date (YYYY-MM-DD)");
    editor.metadata.dirty |= ui
        .text_edit_singleline(&mut editor.statement_date)
        .changed();
    if let Some(value) = difference(editor, accounts) {
        ui.label(format!("Difference: {value}"));
        if value == crate::domain::Money::ZERO {
            ui.colored_label(egui::Color32::GREEN, "Ready — difference is exactly zero.");
        } else {
            ui.colored_label(egui::Color32::YELLOW, "Difference must be exactly zero.");
        }
    }
    ui.small("Previously reconciled transactions remain protected and require explicit confirmation to edit or delete.");
}
