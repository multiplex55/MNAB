//! Persistence-free account register state; commands are committed by services.
use crate::app::view_model::RegisterPageView;
use crate::domain::*;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

pub const MAX_CACHED_PAGES: usize = 3;

/// Whether a row menu should be opened. Native text selection always wins;
/// otherwise right-click selects the row before showing its menu.
#[must_use]
pub const fn row_context_menu(has_native_text_selection: bool) -> bool {
    !has_native_text_selection
}

pub const SHORTCUT_HELP: &[(&str, &str)] = &[
    ("New transaction", "Ctrl+N"),
    ("Edit", "Ctrl+E / F2"),
    ("Commit", "Enter"),
    ("Cancel edit", "Escape"),
    ("Select range", "Shift+Arrow / Shift+Click"),
    ("Add selection", "Ctrl+Click / Space"),
    ("Delete", "Delete"),
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RegisterColumn {
    Selection,
    State,
    Account,
    Date,
    Payee,
    Category,
    Memo,
    Outflow,
    Inflow,
    RunningBalance,
    Approval,
}
pub const COLUMNS: [RegisterColumn; 11] = [
    RegisterColumn::Selection,
    RegisterColumn::State,
    RegisterColumn::Account,
    RegisterColumn::Date,
    RegisterColumn::Payee,
    RegisterColumn::Category,
    RegisterColumn::Memo,
    RegisterColumn::Outflow,
    RegisterColumn::Inflow,
    RegisterColumn::RunningBalance,
    RegisterColumn::Approval,
];
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RegisterView {
    Account(AccountId),
    AllAccounts,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EditField {
    Account,
    Date,
    Payee,
    Category,
    Memo,
    Amount,
}

/// Semantic register intent. Widgets never mutate domain or persistence objects.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RegisterAction {
    Select(TransactionId),
    BeginCreate {
        account_id: AccountId,
    },
    CommitCreate {
        account_id: AccountId,
        draft: TransactionDraft,
    },
    BeginEdit(TransactionId),
    Delete(TransactionId),
}

/// Render an immutable query snapshot and emit only stable-ID actions.
pub fn show(ui: &mut egui::Ui, page: &RegisterPageView, actions: &mut Vec<RegisterAction>) {
    if ui.button("New transaction").clicked() {
        actions.push(RegisterAction::BeginCreate {
            account_id: page.account_id,
        });
    }
    // `show_rows` invokes the closure only for the visible range. A ten-year
    // register therefore creates the same number of widgets as a short one.
    egui::ScrollArea::vertical().show_rows(ui, 24.0, page.rows.len(), |ui, range| {
        egui::Grid::new(("register", page.account_id))
            .striped(true)
            .show(ui, |ui| {
                for heading in [
                    "Date", "Payee", "Category", "Memo", "Outflow", "Inflow", "Balance",
                ] {
                    ui.strong(heading);
                }
                ui.end_row();
                for row in &page.rows[range] {
                    let response = ui.selectable_label(false, row.date.to_string());
                    if response.clicked() {
                        actions.push(RegisterAction::Select(row.transaction_id));
                    }
                    ui.label(&row.payee);
                    ui.label(&row.category);
                    ui.label(row.memo.as_deref().unwrap_or_default());
                    ui.label(if row.amount_cents < 0 {
                        (-row.amount_cents).to_string()
                    } else {
                        String::new()
                    });
                    ui.label(if row.amount_cents >= 0 {
                        row.amount_cents.to_string()
                    } else {
                        String::new()
                    });
                    ui.label(row.running_balance_cents.to_string());
                    ui.end_row();
                }
            });
    });
}

/// Non-transaction rows retained in register ordering for historical inspection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReconciliationSeparator {
    pub reconciliation_id: ReconciliationId,
    pub statement_date: StatementDate,
    pub ending_balance: Money,
    pub state: ReconciliationState,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RegisterRow {
    Transaction(TransactionId),
    Reconciliation(ReconciliationSeparator),
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TransactionDraft {
    pub transaction_id: Option<TransactionId>,
    pub account_id: Option<AccountId>,
    pub date: Option<TransactionDate>,
    pub payee_id: Option<PayeeId>,
    pub category_id: Option<CategoryId>,
    pub memo: String,
    pub amount: Option<Money>,
    pub approval: Option<Approval>,
    pub clearance: Option<Clearance>,
    pub split_lines: Vec<SplitLineDraft>,
}
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SplitLineDraft {
    pub category_id: Option<CategoryId>,
    pub memo: String,
    pub amount: Option<Money>,
}
impl TransactionDraft {
    pub fn set_outflow_inflow(
        &mut self,
        outflow: Money,
        inflow: Money,
    ) -> Result<(), &'static str> {
        if outflow != Money::ZERO && inflow != Money::ZERO {
            return Err("enter either outflow or inflow, not both");
        }
        self.amount = Some(if outflow != Money::ZERO {
            outflow.checked_neg().map_err(|_| "amount overflow")?
        } else {
            inflow
        });
        Ok(())
    }
    #[must_use]
    pub fn outflow_inflow(&self) -> (Money, Money) {
        match self.amount.unwrap_or(Money::ZERO).minor_units() {
            n if n < 0 => (
                self.amount.unwrap().checked_neg().unwrap_or(Money::ZERO),
                Money::ZERO,
            ),
            _ => (Money::ZERO, self.amount.unwrap_or(Money::ZERO)),
        }
    }
    #[must_use]
    pub fn valid(&self) -> bool {
        let body_valid = if self.split_lines.is_empty() {
            self.category_id.is_some()
        } else {
            self.split_lines.len() >= 2
                && self
                    .split_lines
                    .iter()
                    .all(|line| line.category_id.is_some() && line.amount.is_some())
                && self.split_remaining() == Ok(Money::ZERO)
        };
        self.date.is_some() && self.amount.is_some() && self.account_id.is_some() && body_valid
    }
    pub fn split_remaining(&self) -> Result<Money, TransactionError> {
        let lines = self
            .split_lines
            .iter()
            .map(|line| Subtransaction {
                category_id: line.category_id.unwrap_or_default(),
                memo: (!line.memo.is_empty()).then(|| line.memo.clone()),
                amount: line.amount.unwrap_or(Money::ZERO),
            })
            .collect::<Vec<_>>();
        TransactionBody::split_remaining(self.amount.unwrap_or(Money::ZERO), &lines)
    }
    /// Makes the final split line absorb the exact remainder.
    pub fn distribute_remainder(&mut self) -> Result<(), TransactionError> {
        if self.split_lines.len() < 2 {
            return Err(TransactionError::TooFewSplitLines);
        }
        let before_last = self.split_lines[..self.split_lines.len() - 1]
            .iter()
            .try_fold(Money::ZERO, |sum, line| {
                sum.checked_add(line.amount.unwrap_or(Money::ZERO))
            })
            .map_err(|_| TransactionError::SplitOverflow)?;
        self.split_lines.last_mut().unwrap().amount = Some(
            self.amount
                .unwrap_or(Money::ZERO)
                .checked_sub(before_last)
                .map_err(|_| TransactionError::SplitOverflow)?,
        );
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct RegisterState {
    pub view: RegisterView,
    pub selected_id: Option<TransactionId>,
    pub selected_ids: BTreeSet<TransactionId>,
    pub selection_anchor: Option<TransactionId>,
    pub query: String,
    pub edit_field: Option<EditField>,
    /// Drafts are keyed by stable transaction identity (new rows get a newly
    /// allocated identity before persistence) and never borrow page objects.
    pub drafts: BTreeMap<TransactionId, TransactionDraft>,
    pub active_draft_id: Option<TransactionId>,
    originals: BTreeMap<TransactionId, TransactionDraft>,
    pub pages: VecDeque<RegisterPageView>,
    pub deleted_index: Option<usize>,
}
impl RegisterState {
    #[must_use]
    pub fn new(view: RegisterView) -> Self {
        Self {
            view,
            selected_id: None,
            selected_ids: BTreeSet::new(),
            selection_anchor: None,
            query: String::new(),
            edit_field: None,
            drafts: BTreeMap::new(),
            active_draft_id: None,
            originals: BTreeMap::new(),
            pages: VecDeque::new(),
            deleted_index: None,
        }
    }
    pub fn ctrl_n(&mut self) {
        let account_id = match self.view {
            RegisterView::Account(id) => Some(id),
            RegisterView::AllAccounts => None,
        };
        let id = TransactionId::new();
        let draft = TransactionDraft {
            transaction_id: Some(id),
            account_id,
            ..TransactionDraft::default()
        };
        self.originals.insert(id, draft.clone());
        self.drafts.insert(id, draft);
        self.active_draft_id = Some(id);
        self.edit_field = Some(if account_id.is_some() {
            EditField::Date
        } else {
            EditField::Account
        });
    }
    pub fn tab(&mut self) {
        self.edit_field = match self.edit_field {
            Some(EditField::Account) => Some(EditField::Date),
            Some(EditField::Date) => Some(EditField::Payee),
            Some(EditField::Payee) => Some(EditField::Category),
            Some(EditField::Category) => Some(EditField::Memo),
            Some(EditField::Memo) => Some(EditField::Amount),
            value => value,
        };
    }
    pub fn enter(&mut self) -> Option<TransactionDraft> {
        let id = self.active_draft_id?;
        if !self.drafts.get(&id).is_some_and(TransactionDraft::valid) {
            return None;
        }
        self.edit_field = None;
        self.originals.remove(&id);
        self.active_draft_id = None;
        self.drafts.remove(&id)
    }
    pub fn escape(&mut self) {
        if let Some(id) = self.active_draft_id.take() {
            if let Some(original) = self.originals.remove(&id) {
                self.drafts.insert(id, original);
            }
        }
        self.edit_field = None;
    }
    /// Refresh replaces rows externally, while ID selection and the active draft remain untouched.
    pub fn refresh(&mut self, visible_ids: &[TransactionId]) {
        self.selected_ids.retain(|id| visible_ids.contains(id));
        if self
            .selected_id
            .is_some_and(|id| !visible_ids.contains(&id))
        {
            self.selected_id = self.deleted_index.and_then(|old| {
                visible_ids
                    .get(old.min(visible_ids.len().saturating_sub(1)))
                    .copied()
            });
        }
        self.deleted_index = None;
    }
    pub fn select(
        &mut self,
        id: TransactionId,
        ordered_ids: &[TransactionId],
        shift: bool,
        command: bool,
    ) {
        if shift {
            if let Some(anchor) = self
                .selection_anchor
                .and_then(|a| ordered_ids.iter().position(|x| *x == a))
                && let Some(target) = ordered_ids.iter().position(|x| *x == id)
            {
                if !command {
                    self.selected_ids.clear();
                }
                let (start, end) = if anchor <= target {
                    (anchor, target)
                } else {
                    (target, anchor)
                };
                self.selected_ids
                    .extend(ordered_ids[start..=end].iter().copied());
            }
        } else if command {
            if !self.selected_ids.remove(&id) {
                self.selected_ids.insert(id);
            }
            self.selection_anchor = Some(id);
        } else {
            self.selected_ids.clear();
            self.selected_ids.insert(id);
            self.selection_anchor = Some(id);
        }
        self.selected_id = Some(id);
    }
    pub fn mark_deleted(&mut self, ordered_ids: &[TransactionId], id: TransactionId) {
        self.deleted_index = ordered_ids.iter().position(|candidate| *candidate == id);
        self.selected_ids.remove(&id);
    }
    pub fn push_page(&mut self, page: RegisterPageView) {
        self.pages.push_back(page);
        while self.pages.len() > MAX_CACHED_PAGES {
            self.pages.pop_front();
        }
    }
}
