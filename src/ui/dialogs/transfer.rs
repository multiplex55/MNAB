use crate::app::state::{AccountSummary, TransferEditorState};

pub fn show(ui: &mut egui::Ui, editor: &mut TransferEditorState, accounts: &[AccountSummary]) {
    picker(
        ui,
        "From",
        &mut editor.draft.from_account,
        accounts,
        &mut editor.metadata.dirty,
    );
    picker(
        ui,
        "To",
        &mut editor.draft.to_account,
        accounts,
        &mut editor.metadata.dirty,
    );
    ui.label("Amount");
    editor.metadata.dirty |= ui.text_edit_singleline(&mut editor.draft.amount).changed();
    ui.label("Date (YYYY-MM-DD)");
    editor.metadata.dirty |= ui.text_edit_singleline(&mut editor.draft.date).changed();
    ui.label("Memo");
    editor.metadata.dirty |= ui.text_edit_singleline(&mut editor.draft.memo).changed();
    if let Ok(summary) = editor.draft.summary() {
        let name = |id: Option<crate::domain::AccountId>| {
            id.and_then(|id| accounts.iter().find(|a| a.id == id))
                .map_or("Account", |a| a.name.as_str())
        };
        ui.separator();
        ui.strong("Balance effect preview");
        ui.label(format!(
            "{}: {}",
            name(editor.draft.from_account),
            summary.cash_decreases
        ));
        ui.label(format!(
            "{}: {}",
            name(editor.draft.to_account),
            summary.savings_increases
        ));
        if summary.goal_increases != crate::domain::Money::ZERO {
            ui.label(format!("Goal/category: {}", summary.goal_increases));
        }
    }
}

fn picker(
    ui: &mut egui::Ui,
    label: &str,
    selected: &mut Option<crate::domain::AccountId>,
    accounts: &[AccountSummary],
    dirty: &mut bool,
) {
    egui::ComboBox::from_label(label)
        .selected_text(
            selected
                .and_then(|id| accounts.iter().find(|a| a.id == id).map(|a| a.name.clone()))
                .unwrap_or_else(|| "Choose…".into()),
        )
        .show_ui(ui, |ui| {
            for account in accounts.iter().filter(|a| !a.closed) {
                *dirty |= ui
                    .selectable_value(selected, Some(account.id), &account.name)
                    .changed();
            }
        });
}
