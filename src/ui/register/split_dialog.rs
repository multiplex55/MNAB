//! Blocking, draft-local split transaction editor.

use crate::{
    app::state::{AppState, EditorState},
    domain::{CategoryId, Money},
};

#[must_use]
pub fn is_open(state: &AppState) -> bool {
    matches!(&state.editor,
        EditorState::CreatingTransaction(editor) | EditorState::EditingTransaction(editor)
        if editor.split_dialog.is_some())
}

/// Renders after the register so `Modal`'s backdrop owns all pointer interaction.
pub fn show(ctx: &egui::Context, state: &mut AppState) {
    let categories = state
        .category_catalog
        .last_successful
        .as_ref()
        .map(|catalog| {
            catalog
                .groups
                .iter()
                .map(|group| {
                    (
                        group.name.clone(),
                        group
                            .categories
                            .iter()
                            .filter(|category| {
                                !category.hidden
                                    && !category.archived
                                    && !category.protected
                                    && !category.credit_card_payment
                            })
                            .map(|category| (category.id, category.name.clone()))
                            .collect::<Vec<_>>(),
                    )
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let valid_ids = categories
        .iter()
        .flat_map(|(_, entries)| entries.iter().map(|(id, _)| *id))
        .collect::<Vec<_>>();
    let editor = match &mut state.editor {
        EditorState::CreatingTransaction(editor) | EditorState::EditingTransaction(editor) => {
            editor
        }
        _ => return,
    };
    if editor.split_dialog.is_none() {
        return;
    }
    editor.validate_split_dialog(|id| valid_ids.contains(&id));

    let escape = ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
    let enter = ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
    // Consume navigation before ComboBox sees it only when no picker is open. A picker
    // remains the innermost keyboard owner.
    if !ctx.is_popup_open() {
        for key in [egui::Key::ArrowUp, egui::Key::ArrowDown] {
            ctx.input_mut(|input| {
                input.consume_key(egui::Modifiers::NONE, key);
            });
        }
    }
    if escape {
        editor.cancel_split_dialog();
        return;
    }

    let mut cancel = false;
    let mut save = false;
    egui::Modal::new(egui::Id::new((
        "split-transaction-modal",
        editor.transaction_id,
    )))
    .show(ctx, |ui| {
        ui.heading("Split Transaction");
        let total = (|| -> Option<Money> {
            let remaining = editor.split_dialog_remaining().ok()?;
            let mut allocated = Money::ZERO;
            for line in &editor.split_dialog.as_ref()?.lines {
                allocated = allocated
                    .checked_add(
                        crate::app::transaction_editor::parse_split_currency_field(
                            &line.amount_text,
                        )
                        .ok()?,
                    )
                    .ok()?;
            }
            allocated.checked_add(remaining).ok()
        })();
        ui.label(format!(
            "Transaction total: {}",
            total.map_or_else(|| "Invalid amount".into(), |value| value.to_string())
        ));

        let line_count = editor
            .split_dialog
            .as_ref()
            .map_or(0, |dialog| dialog.lines.len());
        let mut remove = None;
        for index in 0..line_count {
            ui.push_id(("split-line", index), |ui| {
                ui.group(|ui| {
                    ui.strong(format!("Split {}", index + 1));
                    let dialog = editor.split_dialog.as_mut().unwrap();
                    let line = &mut dialog.lines[index];
                    let selected = line
                        .category_id
                        .and_then(|id| {
                            categories
                                .iter()
                                .flat_map(|(_, c)| c)
                                .find(|(candidate, _)| *candidate == id)
                        })
                        .map_or("Choose a category", |(_, name)| name);
                    ui.label("Category");
                    egui::ComboBox::from_id_salt("category")
                        .selected_text(selected)
                        .show_ui(ui, |ui| {
                            for (group, entries) in &categories {
                                ui.strong(group); // heading: deliberately not selectable
                                for (id, name) in entries {
                                    ui.indent(group, |ui| {
                                        ui.selectable_value(&mut line.category_id, Some(*id), name);
                                    });
                                }
                            }
                        })
                        .response
                        .on_hover_text(format!("Category for split {}", index + 1));
                    ui.label("Memo");
                    ui.add(egui::TextEdit::singleline(&mut line.memo).id_salt("memo"))
                        .on_hover_text(format!("Memo for split {}", index + 1));
                    ui.label("Amount");
                    ui.add(egui::TextEdit::singleline(&mut line.amount_text).id_salt("amount"))
                        .on_hover_text(format!("Amount for split {}", index + 1));
                    if let Some(Some(error)) = dialog.line_errors.get(index) {
                        ui.colored_label(ui.visuals().error_fg_color, error);
                    }
                    if ui.button(format!("Remove split {}", index + 1)).clicked() {
                        remove = Some(index);
                    }
                });
            });
        }
        if let Some(index) = remove {
            editor.split_dialog.as_mut().unwrap().lines.remove(index);
        }
        ui.horizontal(|ui| {
            if ui.button("Add Split").clicked() {
                editor
                    .split_dialog
                    .as_mut()
                    .unwrap()
                    .lines
                    .push(Default::default());
            }
            if ui.button("Distribute Remaining").clicked() {
                editor.distribute_split_dialog_remaining();
            }
        });
        let remaining = editor.split_dialog_remaining();
        match remaining {
            Ok(value) if value == Money::ZERO => {
                ui.colored_label(
                    ui.visuals().hyperlink_color,
                    format!("Remaining: {value} ✓"),
                );
            }
            Ok(value) => {
                ui.colored_label(ui.visuals().warn_fg_color, format!("Remaining: {value}"));
            }
            Err(error) => {
                ui.colored_label(ui.visuals().error_fg_color, format!("Remaining: {error}"));
            }
        }
        if let Some(error) = editor
            .split_dialog
            .as_ref()
            .and_then(|dialog| dialog.form_error.as_ref())
        {
            ui.colored_label(ui.visuals().error_fg_color, error);
        }
        let enabled = editor.can_save_split_dialog(|id| valid_ids.contains(&id));
        ui.horizontal(|ui| {
            if ui.button("Cancel").clicked() {
                cancel = true;
            }
            if ui
                .add_enabled(enabled, egui::Button::new("Save Splits"))
                .clicked()
            {
                save = true;
            }
        });
        if enter && enabled && !ctx.is_popup_open() {
            save = true;
        }
    });
    if cancel {
        editor.cancel_split_dialog();
    } else if save {
        editor.save_split_dialog(|id: CategoryId| valid_ids.contains(&id));
    } else {
        // Text edits are applied during this frame. Keep the stored, per-line
        // diagnostics synchronized even when Save Splits is disabled.
        editor.validate_split_dialog(|id| valid_ids.contains(&id));
    }
}
