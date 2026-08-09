use crate::{
    app::{
        dispatcher::ActionCollector,
        state::{AppState, EditorMetadata, EditorState},
        transaction_editor::{SplitLineForm, TransactionEditorState},
    },
    domain::{Clearance, Money},
};

pub fn field_id(transaction: Option<crate::domain::TransactionId>, name: &'static str) -> egui::Id {
    egui::Id::new(("transaction-field", transaction, name))
}

pub fn owns_text_focus(
    ctx: &egui::Context,
    transaction: Option<crate::domain::TransactionId>,
) -> bool {
    ctx.memory(egui::Memory::focused).is_some_and(|focused| {
        ["date", "memo", "outflow", "inflow"]
            .into_iter()
            .any(|name| focused == field_id(transaction, name))
    })
}

fn focus_once(
    editor: &mut TransactionEditorState,
    field: crate::app::transaction_editor::TransactionEditorField,
    response: &egui::Response,
) {
    if editor.focus_pending && editor.focus_field == field {
        response.request_focus();
        editor.focus_pending = false;
    }
}

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

pub fn show_cell(
    ui: &mut egui::Ui,
    state: &mut AppState,
    commands: &mut ActionCollector,
    scope: super::RegisterScope,
    column: super::RegisterColumn,
    cached_page_missing: bool,
) {
    debug_assert_eq!(
        state.editor.surface(),
        crate::app::state::EditorSurface::InlineRegister
    );
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
    let identity = editor.transaction_id;
    ui.push_id(("transaction-editor", identity), |ui| {
        ui.add_enabled_ui(enabled, |ui| match column {
            super::RegisterColumn::Selection => {
                ui.vertical(|ui| {
                    if cached_page_missing {
                        ui.colored_label(ui.visuals().warn_fg_color, "⚠")
                            .on_hover_text("The edited transaction is not in the cached page; your changes are preserved.");
                    }
                    if ui.add_enabled(transaction_commit_available(editor), egui::Button::new("💾"))
                        .on_hover_text("Save Transaction (Enter)").clicked() {
                        commands.push(crate::app::command::AppCommand::Commit);
                    }
                    if ui.button("✕").on_hover_text("Cancel (Esc)").clicked() {
                        commands.push(crate::app::command::AppCommand::Cancel);
                    }
                    if !enabled { ui.spinner(); }
                    if editor.metadata.commit_state == crate::app::state::CommitState::Failed
                        && !editor.metadata.validation_errors.is_empty()
                    {
                        ui.colored_label(ui.visuals().error_fg_color, format!(
                            "{} Changes were not lost.", editor.metadata.validation_errors.join(" ")
                        ));
                        if ui.button("Retry").clicked() {
                            commands.push(crate::app::command::AppCommand::Commit);
                        }
                    }
                });
            }
            super::RegisterColumn::Account => {
                if scope == super::RegisterScope::AllTransactions {
                        let name = accounts
                            .iter()
                            .find(|a| Some(a.id) == editor.account_id)
                            .map_or("Choose an account", |a| a.name.as_str());
                        let r = egui::ComboBox::from_id_salt(field_id(identity, "account"))
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
                        focus_once(editor, crate::app::transaction_editor::TransactionEditorField::Account, &r.response);
                        if let Some(error) = &editor.errors.account { ui.colored_label(ui.visuals().error_fg_color, error); }
                }
            }
            super::RegisterColumn::Date => {
                    let date = crate::ui::widgets::date_picker_with_id(ui, &mut editor.date_text, field_id(identity, "date"));
                    focus_once(editor, crate::app::transaction_editor::TransactionEditorField::Date, &date);
                    if date.lost_focus() {
                        editor.normalize_date_on_blur();
                    }
                    if let Some(error) = &editor.errors.date { ui.colored_label(ui.visuals().error_fg_color, error); }
            }
            super::RegisterColumn::PayeeTransfer => {
                    let payee = editor
                        .payee_id
                        .and_then(|id| payee_names.iter().find(|p| p.0 == Some(id)))
                        .map_or("Choose a payee", |p| p.1.as_str());
                    let r = egui::ComboBox::from_id_salt(field_id(identity, "payee"))
                        .selected_text(payee)
                        .show_ui(ui, |ui| {
                            for (id, name) in &payee_names {
                                if let Some(id) = id {
                                    ui.selectable_value(&mut editor.payee_id, Some(*id), name);
                                }
                            }
                        });
                    focus_once(editor, crate::app::transaction_editor::TransactionEditorField::Payee, &r.response);
                    if let Some(error) = &editor.errors.payee { ui.colored_label(ui.visuals().error_fg_color, error); }
            }
            super::RegisterColumn::Category => {
                    let category = editor
                        .category_id
                        .and_then(|id| category_names.iter().find(|c| c.0 == id))
                        .map_or("Choose a category", |c| c.1.as_str());
                    let category_response = egui::ComboBox::from_id_salt(field_id(identity, "category"))
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
                    focus_once(editor, crate::app::transaction_editor::TransactionEditorField::Category, &category_response.response);
                    if let Some(error) = &editor.errors.category { ui.colored_label(ui.visuals().error_fg_color, error); }
                    for (i, split) in editor.splits.iter_mut().enumerate() {
                        ui.horizontal(|ui| {
                            ui.label(format!("Split {}", i + 1));
                            let name = split.category_id.and_then(|id| category_names.iter().find(|c| c.0 == id)).map_or("Choose", |c| c.1.as_str());
                            egui::ComboBox::from_id_salt(("split-category", i)).selected_text(name).show_ui(ui, |ui| {
                                for (id, name, group) in &category_names { ui.selectable_value(&mut split.category_id, Some(*id), format!("{group} / {name}")); }
                            });
                            if let Some(Some(error)) = editor.errors.split_lines.get(i) { ui.colored_label(ui.visuals().error_fg_color, error); }
                        });
                    }
            }
            super::RegisterColumn::Memo => {
                    let memo = ui.add(egui::TextEdit::singleline(&mut editor.memo).id(field_id(identity, "memo")));
                    focus_once(editor, crate::app::transaction_editor::TransactionEditorField::Memo, &memo);
                    if editor.reconciled && !editor.protected_edit_confirmed {
                        ui.colored_label(ui.visuals().warn_fg_color, "Reconciled and protected");
                        if ui.small_button("Edit Anyway").clicked() { editor.protected_edit_confirmed = true; }
                        if let Some(error) = &editor.errors.protected_edit { ui.colored_label(ui.visuals().error_fg_color, error); }
                    }
                    if editor.closed_account && !editor.closed_account_confirmed {
                        ui.colored_label(ui.visuals().warn_fg_color, "Closed account");
                        if ui.small_button("Edit Anyway").clicked() { editor.closed_account_confirmed = true; }
                        if let Some(error) = &editor.errors.closed_account { ui.colored_label(ui.visuals().error_fg_color, error); }
                    }
                    if let Some(error) = &editor.errors.form { ui.colored_label(ui.visuals().error_fg_color, error); }
            }
            super::RegisterColumn::Outflow => {
                    let outflow = ui.add(
                        egui::TextEdit::singleline(&mut editor.outflow_text)
                            .id(field_id(identity, "outflow"))
                            .horizontal_align(egui::Align::RIGHT),
                    );
                    focus_once(editor, crate::app::transaction_editor::TransactionEditorField::Outflow, &outflow);
                    if outflow.lost_focus() {
                        editor.normalize_amount_on_blur(true);
                    }
                    if let Some(error) = &editor.errors.amount { ui.colored_label(ui.visuals().error_fg_color, error); }
            }
            super::RegisterColumn::Inflow => {
                    let inflow = ui.add(
                        egui::TextEdit::singleline(&mut editor.inflow_text)
                            .id(field_id(identity, "inflow"))
                            .horizontal_align(egui::Align::RIGHT),
                    );
                    focus_once(editor, crate::app::transaction_editor::TransactionEditorField::Inflow, &inflow);
                    if inflow.lost_focus() {
                        editor.normalize_amount_on_blur(false);
                    }
            }
            super::RegisterColumn::Cleared => {
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
            }
            super::RegisterColumn::Approved => {
                    let approval_label = if editor.approved {
                        "Approved"
                    } else {
                        "Needs Approval"
                    };
                    ui.checkbox(&mut editor.approved, approval_label)
                        .on_hover_text(approval_label);
            }
            super::RegisterColumn::RunningBalance => { ui.label("—"); }
        });
    });
}
