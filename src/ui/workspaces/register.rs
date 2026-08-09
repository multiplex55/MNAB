use crate::app::transaction_editor::{EditorError, SplitLineForm, TransactionEditorState};
use crate::{
    app::{
        dispatcher::ActionCollector,
        register::{AllMatchingClick, TransactionSelection},
        state::AppState,
    },
    domain::{AccountId, Approval, CategoryId, Clearance, Money, TransactionDate, TransactionId},
};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RegisterColumn {
    Account,
    Date,
    PayeeTransfer,
    Category,
    Memo,
    Outflow,
    Inflow,
    Cleared,
    Approved,
    Balance,
}
pub const ACCOUNT_COLUMNS: &[RegisterColumn] = &[
    RegisterColumn::Date,
    RegisterColumn::PayeeTransfer,
    RegisterColumn::Category,
    RegisterColumn::Memo,
    RegisterColumn::Outflow,
    RegisterColumn::Inflow,
    RegisterColumn::Cleared,
    RegisterColumn::Approved,
];
pub const ALL_TRANSACTION_COLUMNS: &[RegisterColumn] = &[
    RegisterColumn::Account,
    RegisterColumn::Date,
    RegisterColumn::PayeeTransfer,
    RegisterColumn::Category,
    RegisterColumn::Memo,
    RegisterColumn::Outflow,
    RegisterColumn::Inflow,
    RegisterColumn::Cleared,
    RegisterColumn::Approved,
];
impl RegisterColumn {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Account => "Account",
            Self::Date => "Date",
            Self::PayeeTransfer => "Payee / Transfer",
            Self::Category => "Category",
            Self::Memo => "Memo",
            Self::Outflow => "Outflow",
            Self::Inflow => "Inflow",
            Self::Cleared => "Cleared",
            Self::Approved => "Approved",
            Self::Balance => "Balance",
        }
    }
}

pub fn show_register_header(ui: &mut egui::Ui, columns: &[RegisterColumn]) {
    ui.horizontal(|ui| {
        for column in columns {
            ui.strong(column.label());
        }
    });
}

fn format_minor_units(cents: i64) -> String {
    let sign = if cents < 0 { "-" } else { "" };
    let magnitude = cents.unsigned_abs();
    format!("{sign}${}.{:02}", magnitude / 100, magnitude % 100)
}

fn show_page_table(
    ui: &mut egui::Ui,
    state: &AppState,
    page: &crate::app::view_model::RegisterPageView,
    commands: &mut ActionCollector,
) {
    use egui_extras::{Column, TableBuilder};
    let all = matches!(
        page.scope,
        crate::app::view_model::RegisterScope::AllTransactions
    );
    let mut columns = if all {
        ALL_TRANSACTION_COLUMNS.to_vec()
    } else {
        ACCOUNT_COLUMNS.to_vec()
    };
    let mut table = TableBuilder::new(ui)
        .striped(true)
        .resizable(true)
        .sense(egui::Sense::click());
    for column in &columns {
        let width = match column {
            RegisterColumn::Memo => 160.0,
            RegisterColumn::PayeeTransfer | RegisterColumn::Category | RegisterColumn::Account => {
                120.0
            }
            _ => 82.0,
        };
        table = table.column(Column::initial(width).at_least(60.0));
    }
    table
        .header(24.0, |mut header| {
            for column in &columns {
                header.col(|ui| {
                    ui.strong(column.label());
                });
            }
        })
        .body(|body| {
            body.rows(24.0, page.rows.len(), |mut row| {
                let model = &page.rows[row.index()];
                row.set_selected(state.register_selection.contains(model.transaction_id));
                let markers = [
                    model.reconciled.then_some("Reconciled"),
                    model.is_transfer.then_some("Transfer"),
                    (model.split_count > 0).then_some("Split"),
                    model.import_batch_id.is_some().then_some("Imported"),
                    (!model.approved).then_some("Unapproved"),
                ]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" · ");
                for column in &columns {
                    row.col(|ui| match column {
                        RegisterColumn::Account => {
                            ui.label(&model.account_name);
                        }
                        RegisterColumn::Date => {
                            ui.label(model.date.to_string());
                        }
                        RegisterColumn::PayeeTransfer => {
                            ui.label(if markers.is_empty() {
                                model.payee_name.clone()
                            } else {
                                format!("{}  [{markers}]", model.payee_name)
                            });
                        }
                        RegisterColumn::Category => {
                            ui.label(&model.category_name);
                        }
                        RegisterColumn::Memo => {
                            ui.label(model.memo.as_deref().unwrap_or(""));
                        }
                        RegisterColumn::Outflow => {
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.monospace(format_minor_units(model.outflow_cents));
                                },
                            );
                        }
                        RegisterColumn::Inflow => {
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.monospace(format_minor_units(model.inflow_cents));
                                },
                            );
                        }
                        RegisterColumn::Cleared => {
                            ui.label(&model.cleared_state);
                        }
                        RegisterColumn::Approved => {
                            ui.label(if model.approved {
                                "Approved"
                            } else {
                                "Needs approval"
                            });
                        }
                        RegisterColumn::Balance => {
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.monospace(
                                        model
                                            .running_balance_cents
                                            .map_or_else(|| "—".into(), format_minor_units),
                                    );
                                },
                            );
                        }
                    });
                }
                let response = row.response();
                if response.clicked() {
                    let modifiers = response.ctx.input(|i| i.modifiers);
                    commands.push(crate::app::command::ApplicationAction::Register(
                        crate::app::command::RegisterAction::Click {
                            id: model.transaction_id,
                            ctrl: modifiers.command || modifiers.ctrl,
                            shift: modifiers.shift,
                        },
                    ));
                }
                if response.double_clicked() {
                    commands.push(crate::app::command::ApplicationAction::Register(
                        crate::app::command::RegisterAction::BeginEdit(model.transaction_id),
                    ));
                }
            });
        });
    columns.clear();
}

/// Builds an editor from the register projection, including its complete ordered split aggregate.
#[must_use]
pub fn editor_from_row(
    row: &crate::app::view_model::RegisterRowView,
    metadata: crate::app::state::EditorMetadata,
) -> Option<TransactionEditorState> {
    if row.split_count as usize != row.splits.len() {
        return None;
    }
    // A register projection does not contain the complete paired transfer aggregate. Refuse to
    // manufacture an ordinary categorized editor from that lossy projection.
    if row.is_transfer {
        return None;
    }
    let mut editor = TransactionEditorState::new(Some(row.account_id), metadata);
    editor.transaction_id = Some(row.transaction_id);
    editor.date_text = row
        .date
        .format(time::macros::format_description!("[month]/[day]/[year]"))
        .ok()?;
    editor.payee_id = row.payee_id;
    editor.category_id = row.category_id;
    editor.memo = row.memo.clone().unwrap_or_default();
    editor.outflow_text = (row.outflow_cents != 0)
        .then(|| {
            format!(
                "{}.{:02}",
                row.outflow_cents.unsigned_abs() / 100,
                row.outflow_cents.unsigned_abs() % 100
            )
        })
        .unwrap_or_default();
    editor.inflow_text = (row.inflow_cents != 0)
        .then(|| {
            format!(
                "{}.{:02}",
                row.inflow_cents.unsigned_abs() / 100,
                row.inflow_cents.unsigned_abs() % 100
            )
        })
        .unwrap_or_default();
    editor.clearance = match row.cleared_state.to_ascii_lowercase().as_str() {
        "cleared" => Clearance::Cleared,
        "reconciled" => Clearance::Reconciled,
        _ => Clearance::Uncleared,
    };
    editor.approved = row.approved;
    editor.reconciled = row.reconciled;
    editor.splits = row
        .splits
        .iter()
        .map(|split| SplitLineForm {
            category_id: Some(split.category_id),
            amount_text: Money::from_minor_units(split.amount_cents).to_string(),
            memo: split.memo.clone().unwrap_or_default(),
        })
        .collect();
    Some(editor)
}

/// Whether the inline editor can be committed without losing information.
/// Split arithmetic deliberately goes through `TransactionEditor::remaining`, so the UI and the
/// eventual command validation use the same integer-minor-unit model.
#[must_use]
pub fn transaction_commit_available(editor: &TransactionEditorState) -> bool {
    (!editor.reconciled || editor.protected_edit_confirmed)
        && editor.remaining() == Ok(Money::ZERO)
        && editor
            .build_transaction(crate::domain::BudgetId::new())
            .is_ok()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SortDirection {
    Ascending,
    Descending,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RegisterRow {
    pub id: TransactionId,
    pub account_id: AccountId,
    pub date: TransactionDate,
    pub payee: String,
    pub category: Option<String>,
    pub memo: Option<String>,
    pub amount: Money,
    pub cleared: Clearance,
    pub approved: Approval,
    pub balance: Money,
    pub is_transfer: bool,
    pub split_count: usize,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RegisterFilter {
    pub search: String,
    pub account: Option<AccountId>,
    pub category: Option<String>,
    pub approval: Option<Approval>,
    pub clearance: Option<Clearance>,
    pub transfer_only: bool,
    pub uncategorized: bool,
}
impl Default for RegisterFilter {
    fn default() -> Self {
        Self {
            search: String::new(),
            account: None,
            category: None,
            approval: None,
            clearance: None,
            transfer_only: false,
            uncategorized: false,
        }
    }
}

#[derive(Clone, Debug)]
pub struct RegisterState {
    pub selection: TransactionSelection,
    pub sort: (RegisterColumn, SortDirection),
    pub filter: RegisterFilter,
    pub page_size: usize,
    pub loaded: usize,
}
impl Default for RegisterState {
    fn default() -> Self {
        Self {
            selection: TransactionSelection::default(),
            sort: (RegisterColumn::Date, SortDirection::Descending),
            filter: RegisterFilter::default(),
            page_size: 100,
            loaded: 100,
        }
    }
}
impl RegisterState {
    pub fn select(&mut self, id: TransactionId, additive: bool) {
        if additive {
            self.selection.toggle(id, AllMatchingClick::ToggleExclusion);
        } else {
            self.selection.select_only(id);
        }
    }
    pub fn select_range(&mut self, rows: &[RegisterRow], id: TransactionId) {
        let ordered = rows.iter().map(|row| row.id).collect::<Vec<_>>();
        self.selection.select_range(id, &ordered);
    }
    pub fn move_cursor(&mut self, rows: &[RegisterRow], delta: isize, extend: bool) {
        if rows.is_empty() {
            return;
        }
        let current = self
            .selection
            .cursor()
            .and_then(|id| rows.iter().position(|r| r.id == id))
            .unwrap_or(0);
        let next = current.saturating_add_signed(delta).min(rows.len() - 1);
        let id = rows[next].id;
        if extend {
            self.select_range(rows, id);
        } else {
            self.select(id, false);
        }
    }
    pub fn retain_selection(&mut self, refreshed: &[RegisterRow]) {
        let ids: BTreeSet<_> = refreshed.iter().map(|r| r.id).collect();
        if let TransactionSelection::Explicit {
            ids: selected,
            anchor,
            cursor,
        } = &mut self.selection
        {
            selected.retain(|id| ids.contains(id));
            if cursor.is_some_and(|id| !ids.contains(&id)) {
                *cursor = None;
            }
            if anchor.is_some_and(|id| !ids.contains(&id)) {
                *anchor = None;
            }
        }
    }
    pub fn visible_rows(&self, rows: &[RegisterRow]) -> Vec<RegisterRow> {
        let needle = self.filter.search.to_lowercase();
        let mut result: Vec<_> = rows
            .iter()
            .filter(|r| {
                self.filter.account.is_none_or(|v| v == r.account_id)
                    && self.filter.approval.is_none_or(|v| v == r.approved)
                    && self.filter.clearance.is_none_or(|v| v == r.cleared)
                    && (!self.filter.transfer_only || r.is_transfer)
                    && (!self.filter.uncategorized || r.category.is_none())
                    && self
                        .filter
                        .category
                        .as_ref()
                        .is_none_or(|v| r.category.as_ref() == Some(v))
                    && (needle.is_empty()
                        || r.payee.to_lowercase().contains(&needle)
                        || r.memo
                            .as_deref()
                            .unwrap_or("")
                            .to_lowercase()
                            .contains(&needle))
            })
            .cloned()
            .collect();
        result.sort_by(|a, b| {
            let ordering = match self.sort.0 {
                RegisterColumn::Date => a.date.0.cmp(&b.date.0),
                RegisterColumn::PayeeTransfer => a.payee.cmp(&b.payee),
                RegisterColumn::Category => a.category.cmp(&b.category),
                RegisterColumn::Memo => a.memo.cmp(&b.memo),
                RegisterColumn::Outflow | RegisterColumn::Inflow => a.amount.cmp(&b.amount),
                RegisterColumn::Cleared => (a.cleared as u8).cmp(&(b.cleared as u8)),
                RegisterColumn::Approved => (a.approved as u8).cmp(&(b.approved as u8)),
                RegisterColumn::Balance => a.balance.cmp(&b.balance),
                RegisterColumn::Account => a.account_id.cmp(&b.account_id),
            };
            ordering.then(a.id.cmp(&b.id))
        });
        if self.sort.1 == SortDirection::Descending {
            result.reverse();
        }
        result.truncate(self.loaded);
        result
    }
    pub fn load_more(&mut self) {
        self.loaded = self.loaded.saturating_add(self.page_size);
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TransferEditor {
    pub from_account: Option<AccountId>,
    pub to_account: Option<AccountId>,
    pub amount: String,
    pub date: String,
    pub memo: String,
    pub category_id: Option<CategoryId>,
    pub category_effect_account: Option<AccountId>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransferSummary {
    pub cash_decreases: Money,
    pub savings_increases: Money,
    pub goal_increases: Money,
}
impl TransferEditor {
    pub fn default_effect_to_destination(&mut self) {
        if self.category_id.is_some() && self.category_effect_account.is_none() {
            self.category_effect_account = self.to_account;
        }
    }
    pub fn validate(&self) -> Result<(AccountId, AccountId, TransactionDate, Money), EditorError> {
        let from = self
            .from_account
            .ok_or(EditorError::TransferInSplitUnsupported)?;
        let to = self
            .to_account
            .ok_or(EditorError::TransferInSplitUnsupported)?;
        if from == to {
            return Err(EditorError::TransferInSplitUnsupported);
        }
        let date = time::Date::parse(
            self.date.trim(),
            &time::format_description::well_known::Iso8601::DATE,
        )
        .map_err(|_| EditorError::InvalidDate)?;
        let amount = self
            .amount
            .parse::<Money>()
            .map_err(|_| EditorError::InvalidCurrency)?;
        if amount <= Money::ZERO {
            return Err(EditorError::InvalidCurrency);
        }
        Ok((from, to, TransactionDate(date), amount))
    }
    pub fn summary(&self) -> Result<TransferSummary, EditorError> {
        let (_, _, _, amount) = self.validate()?;
        Ok(TransferSummary {
            cash_decreases: amount
                .checked_neg()
                .map_err(|_| EditorError::InvalidCurrency)?,
            savings_increases: amount,
            goal_increases: if self.category_id.is_some()
                && self.category_effect_account == self.to_account
            {
                amount
            } else {
                Money::ZERO
            },
        })
    }
}

pub fn show(
    ui: &mut egui::Ui,
    _state: &mut AppState,
    account_id: AccountId,
    _commands: &mut ActionCollector,
) {
    if let Some(account) = _state.accounts.iter().find(|a| a.id == account_id) {
        let h = crate::ui::account_header::format(
            &account.name,
            account.account_type,
            account.working_balance,
            account.cleared_balance,
        );
        ui.heading(h.name);
        ui.horizontal(|ui| {
            ui.label(format!("Working: {}", h.working));
            ui.label(format!("Cleared: {}", h.cleared));
            ui.label(format!("Uncleared: {}", h.uncleared));
        });
        ui.horizontal(|ui| {
            for (label, command) in [
                (
                    "New Transaction",
                    crate::app::command::AppCommand::AddTransaction,
                ),
                ("Transfer", crate::app::command::AppCommand::CreateTransfer),
                ("Import", crate::app::command::AppCommand::Import),
                (
                    "Reconcile",
                    crate::app::command::AppCommand::ReconcileAccount,
                ),
            ] {
                if ui.button(label).clicked() {
                    _commands.push(command);
                }
            }
        });
    } else {
        ui.heading("Account Transactions");
    }
    show_register_header(ui, ACCOUNT_COLUMNS);
    load_state(
        ui,
        _state,
        "No transactions in this account",
        "Add a transaction, transfer money, or import a statement.",
        _commands,
    );
}

pub fn load_state(
    ui: &mut egui::Ui,
    state: &mut AppState,
    empty_title: &str,
    empty_action: &str,
    commands: &mut ActionCollector,
) {
    let query = &state.register_query;
    if query.refresh_active && query.last_successful.is_none() {
        ui.spinner();
        ui.label("Loading transactions…");
        return;
    }
    if let Some(error) = &query.safe_failure {
        ui.colored_label(
            ui.visuals().error_fg_color,
            format!("Could not load transactions: {error}"),
        );
        if ui.button("Retry").clicked() {
            commands.push(crate::app::command::AppCommand::RetryOperation);
        }
        if let Some(page) = &query.last_successful {
            ui.small("Showing the last successfully loaded data.");
            show_page_table(ui, state, page, commands);
        }
        return;
    }
    match &query.last_successful {
        Some(page) if !page.rows.is_empty() => {
            ui.label(format!("{} transactions", page.total_matches));
            if query.refresh_active {
                ui.spinner();
                ui.small("Refreshing…");
            }
            show_page_table(ui, state, page, commands);
            let has_more = page.has_more;
            // The editor follows the register row area rather than appearing as a detached form
            // above it. Split lines therefore expand downward beneath their parent register row.
            show_inline_editor(ui, state, commands);
            if has_more {
                ui.small("Scroll near the end to load more…");
            }
        }
        _ => {
            ui.strong(empty_title);
            ui.label(empty_action);
            if ui.button("New Transaction").clicked() {
                commands.push(crate::app::command::AppCommand::AddTransaction);
            }
        }
    }
}

fn show_inline_editor(ui: &mut egui::Ui, state: &mut AppState, commands: &mut ActionCollector) {
    let editor = match &mut state.editor {
        crate::app::state::EditorState::CreatingTransaction(editor)
        | crate::app::state::EditorState::EditingTransaction(editor) => editor,
        _ => return,
    };
    ui.group(|ui| {
        if editor.metadata.commit_state == crate::app::state::CommitState::Submitting {
            ui.disable();
        }
        ui.strong(if editor.transaction_id.is_some() {
            "Edit transaction"
        } else {
            "New transaction (not saved)"
        });
        if editor.reconciled && !editor.protected_edit_confirmed {
            ui.colored_label(
                egui::Color32::YELLOW,
                "Warning: editing this reconciled transaction may invalidate the completed reconciliation.",
            );
            ui.label("Review the change carefully; the account may need to be reconciled again.");
            if ui.button("Edit Anyway").clicked() {
                editor.protected_edit_confirmed = true;
            }
        }
        ui.horizontal(|ui| {
            if state.navigation.workspace
                == crate::app::navigation::Workspace::AllTransactions
            {
                ui.label("Account");
                let account = state
                    .accounts
                    .iter()
                    .find(|account| Some(account.id) == editor.account_id)
                    .map_or("Choose an account", |account| account.name.as_str());
                let response = egui::ComboBox::from_id_salt("transaction-account")
                    .selected_text(account)
                    .show_ui(ui, |ui| {
                        for account in &state.accounts {
                            ui.selectable_value(
                                &mut editor.account_id,
                                Some(account.id),
                                &account.name,
                            );
                        }
                    });
                if editor.focus_field
                    == crate::app::transaction_editor::TransactionEditorField::Account
                {
                    response.response.request_focus();
                }
            }
            ui.label("Date");
            ui.text_edit_singleline(&mut editor.date_text);
            ui.label("Payee");
            ui.label(
                editor.payee_id
                    .map_or("—".into(), |id| id.to_string()),
            );
            ui.label("Category");
            ui.label(
                editor.category_id
                    .map_or("—".into(), |id| id.to_string()),
            );
            ui.label("Memo");
            ui.text_edit_singleline(&mut editor.memo);
        });
        ui.horizontal(|ui| {
            ui.label("Outflow");
            ui.text_edit_singleline(&mut editor.outflow_text);
            ui.label("Inflow");
            ui.text_edit_singleline(&mut editor.inflow_text);
            ui.label("Cleared");
            egui::ComboBox::from_id_salt("inline-clearance")
                .selected_text(format!(
                    "{:?}",
                    editor.clearance
                ))
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut editor.clearance,
                        Clearance::Uncleared,
                        "Uncleared",
                    );
                    ui.selectable_value(
                        &mut editor.clearance,
                        Clearance::Cleared,
                        "Cleared",
                    );
                });
            ui.checkbox(&mut editor.approved, "Approved");
            let can_commit = transaction_commit_available(&editor);
            if ui.add_enabled(can_commit, egui::Button::new("Save")).clicked() {
                commands.push(crate::app::command::AppCommand::Commit);
            }
            if ui.button("Cancel").clicked() {
                commands.push(crate::app::command::AppCommand::Cancel);
            }
        });
        ui.separator();
        ui.strong("Split lines");
        let mut remove = None;
        for (index, split) in editor.splits.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                ui.label(format!("{}.", index + 1));
                ui.label("Category");
                ui.label(split.category_id.map_or("Choose a category".into(), |id| id.to_string()));
                ui.label("Amount");
                ui.text_edit_singleline(&mut split.amount_text);
                ui.label("Memo");
                ui.text_edit_singleline(&mut split.memo);
                if ui.small_button("Remove").clicked() { remove = Some(index); }
            });
        }
        if let Some(index) = remove { editor.splits.remove(index); }
        if ui.button("Add split line").clicked() {
            editor.splits.push(SplitLineForm::default());
        }
        match editor.remaining() {
            Ok(remaining) => {
                let balanced = remaining == Money::ZERO;
                ui.colored_label(
                    if balanced { egui::Color32::GREEN } else { ui.visuals().error_fg_color },
                    format!("Remaining: {}", format_minor_units(remaining.minor_units())),
                );
                if !balanced { ui.small("Split amounts must total the transaction exactly before Save is enabled."); }
            }
            Err(error) => { ui.colored_label(ui.visuals().error_fg_color, format!("Split error: {error:?}")); }
        }
        for error in [
            editor.errors.account.as_ref(), editor.errors.date.as_ref(),
            editor.errors.payee.as_ref(), editor.errors.category.as_ref(),
            editor.errors.amount.as_ref(), editor.errors.protected_edit.as_ref(),
            editor.errors.closed_account.as_ref(), editor.errors.form.as_ref(),
        ].into_iter().flatten().chain(editor.errors.split_lines.iter().filter_map(Option::as_ref)) {
            ui.colored_label(ui.visuals().error_fg_color, error);
        }
        if editor.metadata.commit_state == crate::app::state::CommitState::Failed {
            for error in &editor.metadata.validation_errors {
                ui.colored_label(
                    ui.visuals().error_fg_color,
                    format!("{error} (fix and retry)"),
                );
            }
        }
    });
}
