//! Persistence-free account register state; commands are committed by services.
use crate::domain::*;

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
    Date,
    Payee,
    Category,
    Memo,
    Amount,
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
    pub date: Option<TransactionDate>,
    pub payee_id: Option<PayeeId>,
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
        self.date.is_some() && self.amount.is_some() && self.category_id.is_some()
    }
}

#[derive(Clone, Debug)]
pub struct RegisterState {
    pub view: RegisterView,
    pub selected_id: Option<TransactionId>,
    pub query: String,
    pub edit_field: Option<EditField>,
    pub draft: Option<TransactionDraft>,
    original: Option<TransactionDraft>,
}
impl RegisterState {
    #[must_use]
    pub fn new(view: RegisterView) -> Self {
        Self {
            view,
            selected_id: None,
            query: String::new(),
            edit_field: None,
            draft: None,
            original: None,
        }
    }
    pub fn ctrl_n(&mut self) {
        let draft = TransactionDraft::default();
        self.original = Some(draft.clone());
        self.draft = Some(draft);
        self.edit_field = Some(EditField::Date);
    }
    pub fn tab(&mut self) {
        self.edit_field = match self.edit_field {
            Some(EditField::Date) => Some(EditField::Payee),
            Some(EditField::Payee) => Some(EditField::Category),
            Some(EditField::Category) => Some(EditField::Memo),
            Some(EditField::Memo) => Some(EditField::Amount),
            value => value,
        };
    }
    pub fn enter(&mut self) -> Option<TransactionDraft> {
        if !self.draft.as_ref().is_some_and(TransactionDraft::valid) {
            return None;
        }
        self.edit_field = None;
        self.original = None;
        self.draft.take()
    }
    pub fn escape(&mut self) {
        self.draft = self.original.take();
        self.edit_field = None;
    }
    /// Refresh replaces rows externally, while ID selection and the active draft remain untouched.
    pub fn refresh(&mut self, visible_ids: &[TransactionId]) {
        if self.edit_field.is_none()
            && self
                .selected_id
                .is_some_and(|id| !visible_ids.contains(&id))
        {
            self.selected_id = None;
        }
    }
}
