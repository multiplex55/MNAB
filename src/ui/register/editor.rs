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

/// Case-insensitive filtering which retains the lookup projection's stable order and protects
/// the picker from duplicate identities even if a malformed projection is supplied.
pub fn matching_payees<'a>(
    payees: &'a [crate::app::view_model::PayeeLookupItemView],
    search: &str,
) -> Vec<&'a crate::app::view_model::PayeeLookupItemView> {
    let needle = search.trim().to_lowercase();
    let mut seen = std::collections::BTreeSet::new();
    payees
        .iter()
        .filter(|payee| {
            seen.insert(payee.id)
                && (needle.is_empty() || payee.name.to_lowercase().contains(&needle))
        })
        .collect()
}

fn move_payee_highlight(current: usize, count: usize, down: bool) -> usize {
    if count == 0 {
        return 0;
    }
    if down {
        (current + 1).min(count - 1)
    } else {
        current.saturating_sub(1)
    }
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
    let payees = state
        .payee_lookup
        .last_successful
        .as_ref()
        .map_or(&[][..], |lookup| lookup.payees.as_slice());
    let payee_refreshing = state.payee_lookup.refresh_active;
    let payee_failure = state.payee_lookup.safe_failure.clone();
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
                        .and_then(|id| payees.iter().find(|p| p.id == id))
                        .map_or("Choose a payee", |p| p.name.as_str());
                    let r = egui::ComboBox::from_id_salt(field_id(identity, "payee"))
                        .selected_text(payee)
                        .show_ui(ui, |ui| {
                            let search = ui.add(egui::TextEdit::singleline(&mut editor.payee_search)
                                .hint_text("Search payees…"));
                            if search.changed() { editor.payee_highlight = 0; }
                            let matches = matching_payees(payees, &editor.payee_search);
                            if ui.input(|i| i.key_pressed(egui::Key::ArrowDown)) {
                                editor.payee_highlight = move_payee_highlight(editor.payee_highlight, matches.len(), true);
                            }
                            if ui.input(|i| i.key_pressed(egui::Key::ArrowUp)) {
                                editor.payee_highlight = move_payee_highlight(editor.payee_highlight, matches.len(), false);
                            }
                            if ui.input(|i| i.key_pressed(egui::Key::Escape)) { ui.close(); }
                            if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                                if let Some(choice) = matches.get(editor.payee_highlight) {
                                    editor.payee_id = Some(choice.id);
                                    editor.payee_search.clear();
                                    ui.close();
                                }
                            }
                            if matches.is_empty() {
                                ui.weak("No matching payees");
                            }
                            for (index, choice) in matches.into_iter().enumerate() {
                                let selected = editor.payee_id == Some(choice.id);
                                let label = if index == editor.payee_highlight { format!("› {}", choice.name) } else { choice.name.clone() };
                                if ui.selectable_label(selected, label).clicked() {
                                    editor.payee_id = Some(choice.id);
                                    editor.payee_search.clear();
                                    ui.close();
                                }
                            }
                        });
                    focus_once(editor, crate::app::transaction_editor::TransactionEditorField::Payee, &r.response);
                    if payee_refreshing { ui.spinner().on_hover_text("Refreshing payee choices"); }
                    if let Some(error) = &payee_failure { ui.colored_label(ui.visuals().warn_fg_color, "⚠").on_hover_text(error); }
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
                            if ui.selectable_label(false, "Split transaction…").clicked() {
                                editor.open_split_dialog();
                                ui.close();
                            }
                            ui.separator();
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
                    if !editor.splits.is_empty() { ui.label(format!("Split ({} lines)", editor.splits.len())); }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::view_model::PayeeLookupItemView;

    fn payee(name: &str) -> PayeeLookupItemView {
        PayeeLookupItemView {
            id: crate::domain::PayeeId::new(),
            name: name.into(),
        }
    }

    #[test]
    fn matching_is_case_insensitive_stable_and_deduplicated() {
        let alpha = payee("Alpha Market");
        let beta = payee("beta shop");
        let duplicate = PayeeLookupItemView {
            id: alpha.id,
            name: alpha.name.clone(),
        };
        let values = vec![alpha, beta, duplicate];
        let matches = matching_payees(&values, "MARKET");
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].name, "Alpha Market");
        assert_eq!(matching_payees(&values, "").len(), 2);
    }

    #[test]
    fn keyboard_navigation_is_bounded_and_empty_results_are_safe() {
        assert_eq!(move_payee_highlight(0, 3, true), 1);
        assert_eq!(move_payee_highlight(2, 3, true), 2);
        assert_eq!(move_payee_highlight(0, 3, false), 0);
        assert_eq!(move_payee_highlight(4, 0, true), 0);
        assert!(matching_payees(&[payee("Grocer")], "missing").is_empty());
    }

    #[test]
    fn selecting_a_payee_changes_only_the_active_draft() {
        let metadata = EditorMetadata::new(egui::Id::new("draft"));
        let mut active = TransactionEditorState::new(None, metadata.clone());
        let inactive = TransactionEditorState::new(None, metadata);
        let choice = payee("Choice");
        active.payee_id = Some(choice.id);
        assert_eq!(active.payee_id, Some(choice.id));
        assert_eq!(inactive.payee_id, None);
    }
}
