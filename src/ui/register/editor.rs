use crate::{
    app::{
        dispatcher::ActionCollector,
        state::{AppState, EditorMetadata, EditorState},
        transaction_editor::{SplitLineForm, TransactionEditorState},
    },
    domain::{Clearance, Money},
};

#[must_use]
pub fn editor_from_row(
    row: &crate::app::view_model::RegisterRowView,
    metadata: EditorMetadata,
) -> Option<TransactionEditorState> {
    if row.split_count as usize != row.splits.len() || row.is_transfer {
        return None;
    }
    let mut e = TransactionEditorState::new(Some(row.account_id), metadata);
    e.transaction_id = Some(row.transaction_id);
    e.date_text = row
        .date
        .format(time::macros::format_description!("[month]/[day]/[year]"))
        .ok()?;
    e.payee_id = row.payee_id;
    e.category_id = row.category_id;
    e.memo = row.memo.clone().unwrap_or_default();
    e.outflow_text = amount_text(row.outflow_cents);
    e.inflow_text = amount_text(row.inflow_cents);
    e.clearance = match row.cleared_state.to_ascii_lowercase().as_str() {
        "cleared" => Clearance::Cleared,
        "reconciled" => Clearance::Reconciled,
        _ => Clearance::Uncleared,
    };
    e.approved = row.approved;
    e.reconciled = row.reconciled;
    e.splits = row
        .splits
        .iter()
        .map(|s| SplitLineForm {
            category_id: Some(s.category_id),
            amount_text: Money::from_minor_units(s.amount_cents).to_string(),
            memo: s.memo.clone().unwrap_or_default(),
        })
        .collect();
    Some(e)
}
fn amount_text(cents: i64) -> String {
    if cents == 0 {
        String::new()
    } else {
        format!(
            "{}.{:02}",
            cents.unsigned_abs() / 100,
            cents.unsigned_abs() % 100
        )
    }
}
#[must_use]
pub fn transaction_commit_available(e: &TransactionEditorState) -> bool {
    (!e.reconciled || e.protected_edit_confirmed)
        && e.remaining() == Ok(Money::ZERO)
        && e.build_transaction(crate::domain::BudgetId::new()).is_ok()
}

pub fn show(
    ui: &mut egui::Ui,
    state: &mut AppState,
    commands: &mut ActionCollector,
    scope: super::RegisterScope,
) {
    let accounts = &state.accounts;
    let payee_names = state
        .register_query
        .last_successful
        .as_ref()
        .map(|p| {
            p.rows
                .iter()
                .map(|r| (r.payee_id, r.payee_name.clone()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let category_names = state
        .category_catalog
        .last_successful
        .as_ref()
        .map(|c| {
            c.groups
                .iter()
                .flat_map(|group| {
                    group
                        .categories
                        .iter()
                        .filter(|category| {
                            !category.hidden
                                && !category.archived
                                && !category.protected
                                && !category.credit_card_payment
                        })
                        .map(|category| (category.id, category.name.clone(), group.name.clone()))
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let editor = match &mut state.editor {
        EditorState::CreatingTransaction(e) | EditorState::EditingTransaction(e) => e,
        _ => return,
    };
    let enabled = editor.mutations_enabled();
    let identity = super::editor_row_identity(editor.transaction_id);
    let response = ui
        .push_id(identity, |ui| {
            ui.add_enabled_ui(enabled, |ui| {
                ui.horizontal(|ui| {
                    if scope == super::RegisterScope::AllTransactions {
                        let name = accounts
                            .iter()
                            .find(|a| Some(a.id) == editor.account_id)
                            .map_or("Choose an account", |a| a.name.as_str());
                        let r = egui::ComboBox::from_id_salt("account")
                            .selected_text(name)
                            .show_ui(ui, |ui| {
                                for a in accounts {
                                    ui.selectable_value(
                                        &mut editor.account_id,
                                        Some(a.id),
                                        &a.name,
                                    );
                                }
                            });
                        if editor.focus_field
                            == crate::app::transaction_editor::TransactionEditorField::Account
                        {
                            r.response.request_focus();
                        }
                    }
                    let date = crate::ui::widgets::date_picker(ui, &mut editor.date_text);
                    if date.lost_focus() {
                        editor.normalize_date_on_blur();
                    }
                    let payee = editor
                        .payee_id
                        .and_then(|id| payee_names.iter().find(|p| p.0 == Some(id)))
                        .map_or("Choose a payee", |p| p.1.as_str());
                    let r = egui::ComboBox::from_id_salt("payee")
                        .selected_text(payee)
                        .show_ui(ui, |ui| {
                            for (id, name) in &payee_names {
                                if let Some(id) = id {
                                    ui.selectable_value(&mut editor.payee_id, Some(*id), name);
                                }
                            }
                        });
                    if editor.focus_field
                        == crate::app::transaction_editor::TransactionEditorField::Payee
                    {
                        r.response.request_focus();
                    }
                    let category = editor
                        .category_id
                        .and_then(|id| category_names.iter().find(|c| c.0 == id))
                        .map_or("Choose a category", |c| c.1.as_str());
                    egui::ComboBox::from_id_salt("category")
                        .selected_text(category)
                        .show_ui(ui, |ui| {
                            let mut shown_group: Option<&str> = None;
                            for (id, name, group) in &category_names {
                                if shown_group != Some(group) {
                                    ui.strong(group);
                                    shown_group = Some(group);
                                }
                                ui.horizontal(|ui| {
                                    ui.add_space(12.0);
                                    ui.selectable_value(&mut editor.category_id, Some(*id), name);
                                });
                            }
                        });
                    ui.text_edit_singleline(&mut editor.memo);
                    let outflow = ui.add(
                        egui::TextEdit::singleline(&mut editor.outflow_text)
                            .horizontal_align(egui::Align::RIGHT),
                    );
                    if outflow.lost_focus() {
                        editor.normalize_amount_on_blur(true);
                    }
                    let inflow = ui.add(
                        egui::TextEdit::singleline(&mut editor.inflow_text)
                            .horizontal_align(egui::Align::RIGHT),
                    );
                    if inflow.lost_focus() {
                        editor.normalize_amount_on_blur(false);
                    }
                    egui::ComboBox::from_id_salt("clearance")
                        .selected_text(format!("{:?}", editor.clearance))
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut editor.clearance,
                                Clearance::Uncleared,
                                "Uncleared",
                            );
                            ui.selectable_value(
                                &mut editor.clearance,
                                Clearance::Cleared,
                                "✓ Cleared",
                            );
                            if editor.reconciled {
                                ui.label("🔒 Reconciled")
                                    .on_hover_text("Reconciled; use protected edit to change");
                            }
                        });
                    let approval_label = if editor.approved {
                        "Approved"
                    } else {
                        "Needs Approval"
                    };
                    ui.checkbox(&mut editor.approved, approval_label)
                        .on_hover_text(approval_label);
                });
                if let Some(error) = &editor.errors.date {
                    ui.colored_label(ui.visuals().error_fg_color, error);
                }
                if let Some(error) = &editor.errors.amount {
                    ui.colored_label(ui.visuals().error_fg_color, error);
                }
                let mut remove_split = None;
                for (i, split) in editor.splits.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        ui.label(format!("Split {}", i + 1));
                        let name = split
                            .category_id
                            .and_then(|id| category_names.iter().find(|c| c.0 == id))
                            .map_or("Choose a category", |c| c.1.as_str());
                        egui::ComboBox::from_id_salt(("split-category", i))
                            .selected_text(name)
                            .show_ui(ui, |ui| {
                                for (id, name, group) in &category_names {
                                    ui.selectable_value(
                                        &mut split.category_id,
                                        Some(*id),
                                        format!("{group} / {name}"),
                                    );
                                }
                            });
                        ui.add(egui::TextEdit::singleline(&mut split.memo).hint_text("Memo"));
                        ui.add(
                            egui::TextEdit::singleline(&mut split.amount_text)
                                .hint_text("Amount")
                                .horizontal_align(egui::Align::RIGHT),
                        );
                        if ui.small_button("Remove").clicked() {
                            remove_split = Some(i);
                        }
                    });
                    if let Some(Some(error)) = editor.errors.split_lines.get(i) {
                        ui.colored_label(ui.visuals().error_fg_color, error);
                    }
                }
                if let Some(index) = remove_split {
                    editor.remove_split(index);
                }
                ui.horizontal(|ui| {
                    ui.label(match editor.remaining() {
                        Ok(value) => format!("Remaining: {value}"),
                        Err(_) => "Remaining: —".into(),
                    });
                    if ui.button("Add Split").clicked() {
                        editor.add_split();
                    }
                    if ui
                        .add_enabled(
                            !editor.splits.is_empty(),
                            egui::Button::new("Distribute Remainder"),
                        )
                        .clicked()
                    {
                        if let Err(errors) = editor.distribute_remainder() {
                            editor.errors = errors;
                        }
                    }
                });
                if editor.reconciled && !editor.protected_edit_confirmed {
                    ui.group(|ui| {
                        ui.colored_label(
                            ui.visuals().warn_fg_color,
                            "This transaction is reconciled and protected.",
                        );
                        ui.horizontal(|ui| {
                            if ui.button("Cancel").clicked() {
                                commands.push(crate::app::command::AppCommand::Cancel);
                            }
                            if ui.button("Edit Anyway").clicked() {
                                editor.protected_edit_confirmed = true;
                            }
                        });
                    });
                }
                if editor.closed_account && !editor.closed_account_confirmed {
                    ui.group(|ui| {
                        ui.colored_label(
                            ui.visuals().warn_fg_color,
                            "This account is closed. Editing will not reopen it.",
                        );
                        ui.horizontal(|ui| {
                            if ui.button("Cancel").clicked() {
                                commands.push(crate::app::command::AppCommand::Cancel);
                            }
                            if ui.button("Edit Anyway").clicked() {
                                editor.closed_account_confirmed = true;
                            }
                        });
                    });
                }
                if let Some(error) = &editor.errors.form {
                    ui.colored_label(ui.visuals().error_fg_color, error);
                }
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            transaction_commit_available(editor),
                            egui::Button::new("Save"),
                        )
                        .clicked()
                    {
                        commands.push(crate::app::command::AppCommand::Commit);
                    }
                    if ui.button("Cancel").clicked() {
                        commands.push(crate::app::command::AppCommand::Cancel);
                    }
                });
            });
            if !enabled {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Saving…");
                });
            }
        })
        .response;
    if editor.transaction_id.is_none() {
        response.scroll_to_me(Some(egui::Align::Center));
    }
}
