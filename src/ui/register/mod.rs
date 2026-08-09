//! Register layout and framework-independent interaction state.
pub mod editor;
pub mod table;
pub mod toolbar;
pub mod widgets;

use crate::{
    app::{
        register::{AllMatchingClick, TransactionSelection},
        transaction_editor::{EditorError, TransactionEditorField},
    },
    domain::{AccountId, Approval, CategoryId, Clearance, Money, TransactionDate, TransactionId},
};
use std::collections::BTreeSet;

pub use editor::{editor_from_row, transaction_commit_available};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RegisterColumn {
    Selection,
    Account,
    Date,
    PayeeTransfer,
    Category,
    Memo,
    Outflow,
    Inflow,
    Cleared,
    Approved,
    RunningBalance,
}

pub const ACCOUNT_COLUMNS: &[RegisterColumn] = &[
    RegisterColumn::Selection,
    RegisterColumn::Date,
    RegisterColumn::PayeeTransfer,
    RegisterColumn::Category,
    RegisterColumn::Memo,
    RegisterColumn::Outflow,
    RegisterColumn::Inflow,
    RegisterColumn::Cleared,
    RegisterColumn::Approved,
    RegisterColumn::RunningBalance,
];
pub const ALL_TRANSACTION_COLUMNS: &[RegisterColumn] = &[
    RegisterColumn::Selection,
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
    pub const fn label(self) -> &'static str {
        match self {
            Self::Selection => "",
            Self::Account => "Account",
            Self::Date => "Date",
            Self::PayeeTransfer => "Payee / Transfer",
            Self::Category => "Category",
            Self::Memo => "Memo",
            Self::Outflow => "Outflow",
            Self::Inflow => "Inflow",
            Self::Cleared => "Cleared",
            Self::Approved => "Approved",
            Self::RunningBalance => "Running Balance",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RegisterScope {
    Account,
    AllTransactions,
}
pub const fn columns_for(scope: RegisterScope) -> &'static [RegisterColumn] {
    match scope {
        RegisterScope::Account => ACCOUNT_COLUMNS,
        RegisterScope::AllTransactions => ALL_TRANSACTION_COLUMNS,
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum EditorRowIdentity {
    Draft,
    Transaction(TransactionId),
}
pub const fn editor_row_identity(id: Option<TransactionId>) -> EditorRowIdentity {
    match id {
        Some(id) => EditorRowIdentity::Transaction(id),
        None => EditorRowIdentity::Draft,
    }
}
pub const fn editor_visible(surface: crate::app::state::EditorSurface) -> bool {
    matches!(surface, crate::app::state::EditorSurface::InlineRegister)
}
pub const fn initial_focus(scope: RegisterScope) -> TransactionEditorField {
    match scope {
        RegisterScope::Account => TransactionEditorField::Payee,
        RegisterScope::AllTransactions => TransactionEditorField::Account,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RowMenuAction {
    Edit,
    Clear,
    Uncleared,
    Approve,
    Delete,
}
pub fn valid_row_actions(row: &crate::app::view_model::RegisterRowView) -> Vec<RowMenuAction> {
    let mut actions = vec![RowMenuAction::Edit];
    if !row.reconciled {
        if row.cleared_state.eq_ignore_ascii_case("cleared") {
            actions.push(RowMenuAction::Uncleared);
        } else {
            actions.push(RowMenuAction::Clear);
        }
        if !row.approved {
            actions.push(RowMenuAction::Approve);
        }
        actions.push(RowMenuAction::Delete);
    }
    actions
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
#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub struct RegisterFilter {
    pub search: String,
    pub account: Option<AccountId>,
    pub category: Option<String>,
    pub approval: Option<Approval>,
    pub clearance: Option<Clearance>,
    pub transfer_only: bool,
    pub uncategorized: bool,
}
#[derive(Clone, Debug)]
pub struct RegisterState {
    pub selection: TransactionSelection,
    pub sort: (RegisterColumn, SortDirection),
    pub filter: RegisterFilter,
    pub page_size: usize,
    pub loaded: usize,
    pub focus: Option<TransactionId>,
}
impl Default for RegisterState {
    fn default() -> Self {
        Self {
            selection: Default::default(),
            sort: (RegisterColumn::Date, SortDirection::Descending),
            filter: Default::default(),
            page_size: 100,
            loaded: 100,
            focus: None,
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
        self.focus = Some(id);
    }
    pub fn retain_ids(&mut self, refreshed: impl IntoIterator<Item = TransactionId>) {
        let ids: BTreeSet<_> = refreshed.into_iter().collect();
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
        if self.focus.is_some_and(|id| !ids.contains(&id)) {
            self.focus = None;
        }
    }
    pub fn retain_selection(&mut self, refreshed: &[RegisterRow]) {
        self.retain_ids(refreshed.iter().map(|r| r.id));
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scopes_have_exact_columns() {
        assert_eq!(columns_for(RegisterScope::Account), ACCOUNT_COLUMNS);
        assert_eq!(
            columns_for(RegisterScope::AllTransactions),
            ALL_TRANSACTION_COLUMNS
        );
        assert!(ACCOUNT_COLUMNS.contains(&RegisterColumn::RunningBalance));
        assert!(!ALL_TRANSACTION_COLUMNS.contains(&RegisterColumn::RunningBalance));
    }
    #[test]
    fn editor_is_visible_empty_or_populated() {
        use crate::app::state::EditorSurface;
        assert!(editor_visible(EditorSurface::InlineRegister));
        assert!(!editor_visible(EditorSurface::Modal));
        assert!(!editor_visible(EditorSurface::Workspace));
        assert!(!editor_visible(EditorSurface::None));
    }
    #[test]
    fn editor_identity_is_stable() {
        let id = TransactionId::new();
        assert_eq!(editor_row_identity(None), EditorRowIdentity::Draft);
        assert_eq!(
            editor_row_identity(Some(id)),
            EditorRowIdentity::Transaction(id)
        );
    }
    #[test]
    fn focus_depends_on_scope() {
        assert_eq!(
            initial_focus(RegisterScope::Account),
            TransactionEditorField::Payee
        );
        assert_eq!(
            initial_focus(RegisterScope::AllTransactions),
            TransactionEditorField::Account
        );
    }
    #[test]
    fn selection_survives_until_identity_disappears() {
        let keep = TransactionId::new();
        let gone = TransactionId::new();
        let mut s = RegisterState::default();
        s.select(keep, false);
        s.retain_ids([keep, gone]);
        assert!(s.selection.contains(keep));
        assert_eq!(s.focus, Some(keep));
        s.retain_ids([gone]);
        assert!(!s.selection.contains(keep));
        assert_eq!(s.focus, None);
    }
    #[test]
    fn reconciled_rows_gate_mutating_menu_actions() {
        use crate::app::view_model::*;
        let id = TransactionId::new();
        let account = AccountId::new();
        let row = RegisterRowView {
            transaction_id: id,
            account_id: account,
            account_name: "Checking".into(),
            date: time::macros::date!(2026 - 01 - 01),
            created_at: String::new(),
            payee_id: None,
            payee_name: "Payee".into(),
            category_id: None,
            category_name: "Uncategorized".into(),
            memo: None,
            inflow_cents: 0,
            outflow_cents: 100,
            cleared_state: "Reconciled".into(),
            approved: false,
            reconciled: true,
            transfer_id: None,
            is_transfer: false,
            split_count: 0,
            splits: vec![],
            import_batch_id: None,
            import_source: None,
            review_state: None,
            running_balance_cents: Some(0),
        };
        assert_eq!(valid_row_actions(&row), vec![RowMenuAction::Edit]);
        let mut editable = row;
        editable.reconciled = false;
        editable.cleared_state = "Uncleared".into();
        assert_eq!(
            valid_row_actions(&editable),
            vec![
                RowMenuAction::Edit,
                RowMenuAction::Clear,
                RowMenuAction::Approve,
                RowMenuAction::Delete
            ]
        );
    }
}
