use crate::app::state::AccountEditorState;

pub fn show(
    ui: &mut egui::Ui,
    editor: &mut AccountEditorState,
    groups: &[crate::domain::AccountGroup],
) {
    let creating = editor.account_id.is_none();
    ui.label("Name");
    editor.metadata.dirty |= ui.text_edit_singleline(&mut editor.form.name).changed();
    egui::ComboBox::from_label("Type")
        .selected_text(
            editor
                .form
                .account_type
                .map_or("Choose…".into(), |v| format!("{v:?}")),
        )
        .show_ui(ui, |ui| {
            for kind in [
                crate::domain::AccountType::Checking,
                crate::domain::AccountType::Savings,
                crate::domain::AccountType::Cash,
                crate::domain::AccountType::CreditCard,
                crate::domain::AccountType::Loan,
                crate::domain::AccountType::Asset,
                crate::domain::AccountType::Liability,
            ] {
                editor.metadata.dirty |= ui
                    .selectable_value(
                        &mut editor.form.account_type,
                        Some(kind),
                        format!("{kind:?}"),
                    )
                    .changed();
            }
        });
    if creating {
        ui.label("Opening balance (positive magnitude)");
        editor.metadata.dirty |= ui
            .text_edit_singleline(&mut editor.form.opening_magnitude)
            .changed();
        ui.label("Opening date (YYYY-MM-DD)");
        editor.metadata.dirty |= ui
            .text_edit_singleline(&mut editor.form.opening_date)
            .changed();
    }
    egui::ComboBox::from_label("Group")
        .selected_text(
            editor
                .form
                .group_id
                .and_then(|id| groups.iter().find(|g| g.id == id).map(|g| g.name.clone()))
                .unwrap_or_else(|| "Ungrouped".into()),
        )
        .show_ui(ui, |ui| {
            editor.metadata.dirty |= ui
                .selectable_value(&mut editor.form.group_id, None, "Ungrouped")
                .changed();
            for group in groups {
                editor.metadata.dirty |= ui
                    .selectable_value(&mut editor.form.group_id, Some(group.id), &group.name)
                    .changed();
            }
        });
    ui.label("Note");
    editor.metadata.dirty |= ui.text_edit_multiline(&mut editor.form.note).changed();
    editor.metadata.dirty |= ui.checkbox(&mut editor.form.favorite, "Favorite").changed();
}
