//! Authoritative presentation state and validation for the transaction editor.

use crate::{
    app::state::EditorMetadata,
    domain::{
        AccountId, Approval, BudgetId, CategoryId, Clearance, Money, PayeeId, Subtransaction,
        Transaction, TransactionBody, TransactionDate, TransactionId,
    },
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EditorError {
    DateRequired,
    InvalidDate,
    InvalidCurrency,
    BothOutflowAndInflow,
    SplitCategoryRequired,
    SplitTotalMismatch,
    ClosedAccountConfirmation,
    ReconciledProtectedEdit,
    TransferInSplitUnsupported,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SplitLineForm {
    pub category_id: Option<CategoryId>,
    pub amount_text: String,
    pub memo: String,
}

/// Ephemeral split editor.  The parent fields are retained so cancellation can be
/// lossless even if a future view accidentally changes them while the modal is open.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SplitDialogState {
    pub lines: Vec<SplitLineForm>,
    pub line_errors: Vec<Option<String>>,
    pub form_error: Option<String>,
    pub focused_control: usize,
    pub category_picker: Option<usize>,
    original_category_id: Option<CategoryId>,
    original_splits: Vec<SplitLineForm>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransactionEditorField {
    Account,
    Date,
    Payee,
    Category,
    Memo,
    Outflow,
    Inflow,
    Split(usize),
    Form,
}

/// Deterministic keyboard order used by Tab and Shift+Tab in the register editor.
pub const TRANSACTION_FIELD_ORDER: &[TransactionEditorField] = &[
    TransactionEditorField::Account,
    TransactionEditorField::Date,
    TransactionEditorField::Payee,
    TransactionEditorField::Category,
    TransactionEditorField::Memo,
    TransactionEditorField::Outflow,
    TransactionEditorField::Inflow,
    TransactionEditorField::Form,
];

#[must_use]
pub fn traverse_field(
    current: TransactionEditorField,
    backwards: bool,
    include_account: bool,
) -> TransactionEditorField {
    let fields: Vec<_> = TRANSACTION_FIELD_ORDER
        .iter()
        .copied()
        .filter(|field| include_account || *field != TransactionEditorField::Account)
        .collect();
    let position = fields
        .iter()
        .position(|field| *field == current)
        .unwrap_or(0);
    let next = if backwards {
        position.checked_sub(1).unwrap_or(fields.len() - 1)
    } else {
        (position + 1) % fields.len()
    };
    fields[next]
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TransactionEditorErrors {
    pub account: Option<String>,
    pub date: Option<String>,
    pub payee: Option<String>,
    pub category: Option<String>,
    pub amount: Option<String>,
    pub split_lines: Vec<Option<String>>,
    pub protected_edit: Option<String>,
    pub closed_account: Option<String>,
    pub form: Option<String>,
}
impl TransactionEditorErrors {
    pub fn is_empty(&self) -> bool {
        self.account.is_none()
            && self.date.is_none()
            && self.payee.is_none()
            && self.category.is_none()
            && self.amount.is_none()
            && self.split_lines.iter().all(Option::is_none)
            && self.protected_edit.is_none()
            && self.closed_account.is_none()
            && self.form.is_none()
    }
    /// Fixed visual/tab order makes focus restoration independent of validation implementation.
    pub fn first_invalid_field(&self) -> Option<TransactionEditorField> {
        if self.account.is_some() {
            return Some(TransactionEditorField::Account);
        }
        if self.date.is_some() {
            return Some(TransactionEditorField::Date);
        }
        if self.payee.is_some() {
            return Some(TransactionEditorField::Payee);
        }
        if self.category.is_some() {
            return Some(TransactionEditorField::Category);
        }
        if self.amount.is_some() {
            return Some(TransactionEditorField::Outflow);
        }
        if let Some(i) = self.split_lines.iter().position(Option::is_some) {
            return Some(TransactionEditorField::Split(i));
        }
        if self.protected_edit.is_some() || self.closed_account.is_some() || self.form.is_some() {
            return Some(TransactionEditorField::Form);
        }
        None
    }
}

#[derive(Clone, Debug)]
pub struct TransactionEditorState {
    pub transaction_id: Option<TransactionId>,
    pub account_id: Option<AccountId>,
    pub date_text: String,
    pub outflow_text: String,
    pub inflow_text: String,
    pub payee_id: Option<PayeeId>,
    /// Type-ahead text and highlighted result belong to this draft, not to the register.
    pub payee_search: String,
    pub payee_highlight: usize,
    pub category_id: Option<CategoryId>,
    pub memo: String,
    pub clearance: Clearance,
    pub approved: bool,
    pub splits: Vec<SplitLineForm>,
    pub split_dialog: Option<SplitDialogState>,
    /// Complete source record; properties outside this form are copied from it on save.
    pub original: Option<Transaction>,
    pub reconciled: bool,
    pub closed_account: bool,
    pub protected_edit_confirmed: bool,
    pub closed_account_confirmed: bool,
    pub errors: TransactionEditorErrors,
    /// Field which should receive keyboard focus on the next frame.
    pub focus_field: TransactionEditorField,
    /// Set whenever focus is redirected. The view clears it after the owning widget mounts.
    pub focus_pending: bool,
    pub metadata: EditorMetadata,
}
impl TransactionEditorState {
    pub fn new(account_id: Option<AccountId>, metadata: EditorMetadata) -> Self {
        Self {
            transaction_id: None,
            account_id,
            date_text: String::new(),
            outflow_text: String::new(),
            inflow_text: String::new(),
            payee_id: None,
            payee_search: String::new(),
            payee_highlight: 0,
            category_id: None,
            memo: String::new(),
            clearance: Clearance::Uncleared,
            approved: false,
            splits: vec![],
            split_dialog: None,
            original: None,
            reconciled: false,
            closed_account: false,
            protected_edit_confirmed: false,
            closed_account_confirmed: false,
            errors: TransactionEditorErrors::default(),
            focus_field: if account_id.is_some() {
                TransactionEditorField::Payee
            } else {
                TransactionEditorField::Account
            },
            focus_pending: true,
            metadata,
        }
    }
    pub fn cancel_protected_confirmations(&mut self) {
        self.split_dialog = None;
        self.protected_edit_confirmed = false;
        self.closed_account_confirmed = false;
    }

    /// Opens a detached working copy; no transaction-draft field is changed.
    pub fn open_split_dialog(&mut self) {
        let mut lines = self.splits.clone();
        if lines.is_empty() {
            if let (Some(category_id), Ok(amount)) = (self.category_id, self.transaction_amount())
                && amount != Money::ZERO
            {
                lines.push(SplitLineForm {
                    category_id: Some(category_id),
                    amount_text: amount.to_string(),
                    memo: String::new(),
                });
                lines.push(SplitLineForm::default());
            }
        }
        self.split_dialog = Some(SplitDialogState {
            line_errors: vec![None; lines.len()],
            lines,
            form_error: None,
            focused_control: 0,
            category_picker: None,
            original_category_id: self.category_id,
            original_splits: self.splits.clone(),
        });
    }

    pub fn cancel_split_dialog(&mut self) {
        if let Some(dialog) = self.split_dialog.take() {
            self.category_id = dialog.original_category_id;
            self.splits = dialog.original_splits;
        }
    }

    fn transaction_amount(&self) -> Result<Money, &'static str> {
        let out = parse_currency_field(&self.outflow_text)?;
        let input = parse_currency_field(&self.inflow_text)?;
        if out != Money::ZERO && input != Money::ZERO {
            return Err("Enter either an outflow or an inflow, not both");
        }
        input
            .checked_add(out.checked_neg().map_err(|_| "Amount overflowed")?)
            .map_err(|_| "Amount overflowed")
    }

    pub fn split_dialog_remaining(&self) -> Result<Money, &'static str> {
        let total = self.transaction_amount()?;
        let dialog = self.split_dialog.as_ref().ok_or("Split dialog is closed")?;
        let mut allocated = Money::ZERO;
        for line in &dialog.lines {
            allocated = allocated
                .checked_add(parse_split_currency_field(&line.amount_text)?)
                .map_err(|_| "Split total overflowed")?;
        }
        total
            .checked_add(
                allocated
                    .checked_neg()
                    .map_err(|_| "Split total overflowed")?,
            )
            .map_err(|_| "Split total overflowed")
    }

    /// Validates and applies the working copy to the draft only.  Persistence remains
    /// exclusively the responsibility of the parent editor's Commit command.
    pub fn save_split_dialog(&mut self, category_exists: impl Fn(CategoryId) -> bool) -> bool {
        let Some(mut dialog) = self.split_dialog.take() else {
            return false;
        };
        dialog.line_errors = validate_split_lines(&dialog.lines, &category_exists);
        dialog.form_error = if dialog.lines.len() < 2 {
            Some("A split requires at least two lines".into())
        } else if self.split_dialog_remaining_for(&dialog.lines) != Ok(Money::ZERO) {
            Some("The remaining amount must be exactly zero".into())
        } else {
            None
        };
        if dialog.line_errors.iter().any(Option::is_some) || dialog.form_error.is_some() {
            self.split_dialog = Some(dialog);
            return false;
        }
        self.splits = dialog.lines;
        self.category_id = None;
        true
    }

    #[must_use]
    pub fn can_save_split_dialog(&self, category_exists: impl Fn(CategoryId) -> bool) -> bool {
        let Some(dialog) = &self.split_dialog else {
            return false;
        };
        dialog.lines.len() >= 2
            && dialog.lines.iter().all(|line| {
                line.category_id.is_some_and(&category_exists)
                    && parse_split_currency_field(&line.amount_text).is_ok()
            })
            && self.split_dialog_remaining() == Ok(Money::ZERO)
    }

    /// Refreshes the presentation errors while the user edits. This is separate from
    /// saving so a disabled Save button never hides the reason it is disabled.
    pub fn validate_split_dialog(&mut self, category_exists: impl Fn(CategoryId) -> bool) {
        let Some(dialog) = self.split_dialog.as_mut() else {
            return;
        };
        dialog.line_errors = validate_split_lines(&dialog.lines, category_exists);
    }

    pub fn distribute_split_dialog_remaining(&mut self) -> bool {
        let Ok(remainder) = self.split_dialog_remaining() else {
            return false;
        };
        let Some(last) = self
            .split_dialog
            .as_mut()
            .and_then(|dialog| dialog.lines.last_mut())
        else {
            return false;
        };
        let Ok(current) = parse_split_currency_field(&last.amount_text) else {
            return false;
        };
        let Ok(value) = current.checked_add(remainder) else {
            return false;
        };
        last.amount_text = value.to_string();
        true
    }

    fn split_dialog_remaining_for(&self, lines: &[SplitLineForm]) -> Result<Money, &'static str> {
        let mut remaining = self.transaction_amount()?;
        for line in lines {
            remaining = remaining
                .checked_add(
                    parse_split_currency_field(&line.amount_text)?
                        .checked_neg()
                        .map_err(|_| "Split total overflowed")?,
                )
                .map_err(|_| "Split total overflowed")?;
        }
        Ok(remaining)
    }
    pub fn move_focus(&mut self, backwards: bool) {
        self.focus_field = traverse_field(self.focus_field, backwards, self.account_id.is_none());
        self.focus_pending = true;
    }

    /// Blur is the only point at which valid keyboard input is rewritten.
    pub fn normalize_date_on_blur(&mut self) -> bool {
        let Ok(date) = parse_transaction_date(&self.date_text) else {
            return false;
        };
        self.date_text = date
            .0
            .format(time::macros::format_description!("[month]/[day]/[year]"))
            .expect("fixed date format");
        true
    }

    pub fn normalize_amount_on_blur(&mut self, outflow: bool) -> bool {
        let text = if outflow {
            &mut self.outflow_text
        } else {
            &mut self.inflow_text
        };
        let Ok(amount) = parse_currency_field(text) else {
            return false;
        };
        *text = if text.trim().is_empty() {
            String::new()
        } else {
            amount.to_string()
        };
        true
    }

    #[must_use]
    pub fn mutations_enabled(&self) -> bool {
        self.metadata.commit_state != crate::app::state::CommitState::Submitting
    }
    pub fn remaining(&self) -> Result<Money, TransactionEditorErrors> {
        let (_, amount, lines) = self.validate_parts(false)?;
        TransactionBody::split_remaining(amount, &lines)
            .map_err(|_| form_error("Split total overflowed"))
    }
    /// Adds a blank, identity-bearing split form.  A blank category is deliberate: the
    /// line cannot be persisted until the user chooses a stable `CategoryId`.
    pub fn add_split(&mut self) {
        if self.splits.is_empty() {
            self.splits.push(SplitLineForm::default());
        }
        self.splits.push(SplitLineForm::default());
        self.category_id = None;
    }

    pub fn remove_split(&mut self, index: usize) {
        if index < self.splits.len() {
            self.splits.remove(index);
        }
    }

    /// Uses the domain's checked-cent implementation and writes the exact remainder to
    /// the final line. No allocation/division (and therefore no floating point) occurs.
    pub fn distribute_remainder(&mut self) -> Result<(), TransactionEditorErrors> {
        let (_, parent, mut lines) = self.validate_parts(false)?;
        TransactionBody::distribute_remainder(parent, &mut lines)
            .map_err(|error| form_error(&error.to_string()))?;
        for (form, line) in self.splits.iter_mut().zip(lines) {
            form.amount_text = line.amount.to_string();
        }
        Ok(())
    }
    fn validate_parts(
        &self,
        check_total: bool,
    ) -> Result<(TransactionDate, Money, Vec<Subtransaction>), TransactionEditorErrors> {
        let mut e = TransactionEditorErrors {
            split_lines: vec![None; self.splits.len()],
            ..Default::default()
        };
        if self.account_id.is_none() {
            e.account = Some("Account is required".into());
        }
        let date = match parse_transaction_date(&self.date_text) {
            Ok(v) => Some(v),
            Err(m) => {
                e.date = Some(m.into());
                None
            }
        };
        let out = match parse_currency_field(&self.outflow_text) {
            Ok(v) => Some(v),
            Err(m) => {
                e.amount = Some(m.into());
                None
            }
        };
        let input = match parse_currency_field(&self.inflow_text) {
            Ok(v) => Some(v),
            Err(m) => {
                e.amount = Some(m.into());
                None
            }
        };
        if out.is_some_and(|v| v != Money::ZERO) && input.is_some_and(|v| v != Money::ZERO) {
            e.amount = Some("Enter either an outflow or an inflow, not both".into());
        }
        if self.reconciled && !self.protected_edit_confirmed {
            e.protected_edit = Some("Confirm editing this reconciled transaction".into());
        }
        if self.closed_account && !self.closed_account_confirmed {
            e.closed_account = Some("Confirm editing this closed account".into());
        }
        if self
            .original
            .as_ref()
            .is_some_and(|t| matches!(t.body, TransactionBody::Transfer { .. }))
        {
            e.form =
                Some("Transfer transactions cannot be edited as categorized transactions".into());
        }
        let amount = match (out, input) {
            (Some(o), Some(i)) => o.checked_neg().and_then(|o| i.checked_add(o)).ok(),
            _ => None,
        };
        let mut lines = Vec::new();
        for (i, line) in self.splits.iter().enumerate() {
            let category = match line.category_id {
                Some(v) => v,
                None => {
                    e.split_lines[i] = Some("Category is required".into());
                    continue;
                }
            };
            match parse_split_currency_field(&line.amount_text) {
                Ok(amount) => lines.push(Subtransaction {
                    category_id: category,
                    amount,
                    memo: trimmed(&line.memo),
                }),
                Err(m) => e.split_lines[i] = Some(m.into()),
            }
        }
        if self.splits.is_empty() && self.category_id.is_none() {
            e.category = Some("Category is required".into());
        }
        if !self.splits.is_empty() && self.splits.len() < 2 {
            e.form = Some("A split requires at least two lines".into());
        }
        if check_total
            && !self.splits.is_empty()
            && e.form.is_none()
            && lines.len() == self.splits.len()
            && amount.is_some()
        {
            if let Err(x) = TransactionBody::split(amount.unwrap(), lines.clone()) {
                e.form = Some(x.to_string());
            }
        }
        if !e.is_empty() {
            return Err(e);
        }
        Ok((date.unwrap(), amount.unwrap(), lines))
    }
    pub fn build_transaction(
        &self,
        budget_id: BudgetId,
    ) -> Result<Transaction, TransactionEditorErrors> {
        let (date, amount, lines) = self.validate_parts(true)?;
        let body = if lines.is_empty() {
            TransactionBody::categorized(self.category_id.unwrap())
        } else {
            TransactionBody::split(amount, lines).map_err(|x| form_error(&x.to_string()))?
        };
        let mut result = self.original.clone().unwrap_or(Transaction {
            id: self.transaction_id.unwrap_or_else(TransactionId::new),
            budget_id,
            account_id: self.account_id.unwrap(),
            date,
            payee_id: self.payee_id,
            amount,
            memo: None,
            clearance: self.clearance,
            approval: Approval::Unapproved,
            body: body.clone(),
            archived: false,
            voided: false,
        });
        result.id = self.transaction_id.unwrap_or(result.id);
        if self.original.is_none() {
            result.budget_id = budget_id;
        }
        result.account_id = self.account_id.unwrap();
        result.date = date;
        result.payee_id = self.payee_id;
        result.amount = amount;
        result.memo = trimmed(&self.memo);
        result.clearance = self.clearance;
        result.approval = if self.approved {
            Approval::Approved
        } else {
            Approval::Unapproved
        };
        result.body = body;
        Ok(result)
    }
}

fn validate_split_lines(
    lines: &[SplitLineForm],
    category_exists: impl Fn(CategoryId) -> bool,
) -> Vec<Option<String>> {
    lines
        .iter()
        .map(|line| match line.category_id {
            None => Some("Category is required".into()),
            Some(id) if !category_exists(id) => Some("Category is unavailable".into()),
            Some(_) => parse_split_currency_field(&line.amount_text)
                .err()
                .map(str::to_owned),
        })
        .collect()
}
fn trimmed(s: &str) -> Option<String> {
    (!s.trim().is_empty()).then(|| s.trim().to_owned())
}
fn form_error(message: &str) -> TransactionEditorErrors {
    TransactionEditorErrors {
        form: Some(message.into()),
        ..Default::default()
    }
}

pub fn parse_transaction_date(text: &str) -> Result<TransactionDate, &'static str> {
    let s = text.trim();
    if s.is_empty() {
        return Err("Date is required");
    }
    let us = time::macros::format_description!("[month]/[day]/[year]");
    time::Date::parse(s, us)
        .or_else(|_| time::Date::parse(s, &time::format_description::well_known::Iso8601::DATE))
        .map(TransactionDate)
        .map_err(|_| "Invalid date")
}
pub fn parse_currency_field(text: &str) -> Result<Money, &'static str> {
    let s = text.trim();
    if s.is_empty() {
        return Ok(Money::ZERO);
    }
    if s.starts_with('-') || s.starts_with("$-") || s.starts_with('+') {
        return Err("Enter a non-negative amount");
    }
    s.parse().map_err(|_| "Invalid amount")
}

/// Split amounts are signed because a split may contain refunds/credits and an outflow
/// parent necessarily has a negative cent total. Unlike the two-column parent controls,
/// there is no separate inflow/outflow field from which to infer the sign.
pub fn parse_split_currency_field(text: &str) -> Result<Money, &'static str> {
    let s = text.trim();
    if s.is_empty() {
        return Ok(Money::ZERO);
    }
    s.parse().map_err(|_| "Invalid amount")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::TransferId;
    use time::Month;

    fn editor() -> TransactionEditorState {
        let mut e = TransactionEditorState::new(
            Some(AccountId::new()),
            EditorMetadata::new(egui::Id::new("test")),
        );
        e.date_text = "08/09/2026".into();
        e.category_id = Some(CategoryId::new());
        e
    }
    #[test]
    fn dates_preserve_distinct_empty_invalid_and_accept_us_and_iso() {
        assert_eq!(parse_transaction_date(""), Err("Date is required"));
        assert_eq!(parse_transaction_date("02/30/2025"), Err("Invalid date"));
        for input in ["12/31/2025", "2025-12-31"] {
            let d = parse_transaction_date(input).unwrap();
            assert_eq!((d.0.month(), d.0.day()), (Month::December, 31));
        }
    }
    #[test]
    fn currency_forms_are_exact_and_checked() {
        for (s, cents) in [
            ("", 0),
            ("25", 2500),
            ("25.5", 2550),
            ("25.50", 2550),
            ("$25.50", 2550),
            ("1,234.56", 123456),
        ] {
            assert_eq!(parse_currency_field(s).unwrap().minor_units(), cents);
        }
        for s in ["-1", "$-1", "1,23.00", "1.234", "hello"] {
            assert!(parse_currency_field(s).is_err(), "{s}");
        }
        assert_eq!(
            parse_currency_field("92233720368547758.07")
                .unwrap()
                .minor_units(),
            i64::MAX
        );
        assert!(parse_currency_field("92233720368547758.08").is_err());
    }

    #[test]
    fn split_dialog_open_cancel_and_save_are_draft_local() {
        let mut e = editor();
        let category = e.category_id.unwrap();
        e.outflow_text = "10.01".into();
        e.open_split_dialog();
        let dialog = e.split_dialog.as_ref().unwrap();
        assert_eq!(e.category_id, Some(category));
        assert!(e.splits.is_empty());
        assert_eq!(dialog.lines.len(), 2);
        assert_eq!(dialog.lines[0].category_id, Some(category));
        assert_eq!(dialog.lines[0].amount_text, "-$10.01");

        e.split_dialog.as_mut().unwrap().lines[0].memo = "temporary".into();
        e.cancel_split_dialog();
        assert_eq!(e.category_id, Some(category));
        assert!(e.splits.is_empty());

        e.open_split_dialog();
        let second = CategoryId::new();
        let lines = &mut e.split_dialog.as_mut().unwrap().lines;
        lines[0].amount_text = "-6.00".into();
        lines[1].category_id = Some(second);
        lines[1].amount_text = "-4.01".into();
        assert_eq!(e.split_dialog_remaining(), Ok(Money::ZERO));
        assert!(e.save_split_dialog(|id| id == category || id == second));
        assert_eq!(e.category_id, None);
        assert_eq!(e.splits.len(), 2);
        // Applying splits only modifies the form; persistence still requires build/Commit.
        assert_eq!(
            e.metadata.commit_state,
            crate::app::state::CommitState::Editing
        );
        assert!(e.build_transaction(BudgetId::new()).is_ok());
    }

    #[test]
    fn existing_splits_are_copied_and_validation_is_exact() {
        let mut e = editor();
        e.inflow_text = "0.03".into();
        e.splits = ["0.01", "0.02"]
            .map(|amount| SplitLineForm {
                category_id: Some(CategoryId::new()),
                amount_text: amount.into(),
                memo: String::new(),
            })
            .into();
        let original = e.splits.clone();
        e.open_split_dialog();
        e.split_dialog.as_mut().unwrap().lines[0].amount_text = "0.02".into();
        assert_eq!(
            e.splits, original,
            "the working copy must not alias its parent"
        );
        assert_eq!(e.split_dialog_remaining(), Ok(Money::from_minor_units(-1)));
        assert!(!e.can_save_split_dialog(|_| true));
        e.cancel_protected_confirmations();
        assert!(
            e.split_dialog.is_none(),
            "closing the parent clears its modal"
        );
    }

    #[test]
    fn invalid_split_category_amount_and_remainder_disable_save() {
        let mut e = editor();
        e.outflow_text = "1.00".into();
        e.open_split_dialog();
        assert!(!e.can_save_split_dialog(|_| true)); // blank category and remainder
        let category = e.category_id.unwrap();
        let lines = &mut e.split_dialog.as_mut().unwrap().lines;
        lines[1].category_id = Some(category);
        lines[1].amount_text = "not money".into();
        let _ = lines;
        assert!(!e.can_save_split_dialog(|_| true));
        e.split_dialog.as_mut().unwrap().lines[1].amount_text = "0".into();
        assert!(!e.can_save_split_dialog(|_| false)); // unavailable category
    }

    #[test]
    fn split_dialog_validation_is_available_before_save() {
        let mut e = editor();
        e.outflow_text = "1.00".into();
        e.open_split_dialog();
        e.validate_split_dialog(|_| true);
        assert_eq!(
            e.split_dialog.as_ref().unwrap().line_errors,
            vec![None, Some("Category is required".into())]
        );

        let line = &mut e.split_dialog.as_mut().unwrap().lines[1];
        line.category_id = Some(CategoryId::new());
        line.amount_text = "not money".into();
        e.validate_split_dialog(|_| true);
        assert_eq!(
            e.split_dialog.as_ref().unwrap().line_errors[1].as_deref(),
            Some("Invalid amount")
        );
    }
    #[test]
    fn amount_signs_and_required_fields_are_structured() {
        let mut e = editor();
        e.outflow_text = "25.50".into();
        assert_eq!(
            e.build_transaction(BudgetId::new())
                .unwrap()
                .amount
                .minor_units(),
            -2550
        );
        e.outflow_text.clear();
        e.inflow_text = "25.50".into();
        assert_eq!(
            e.build_transaction(BudgetId::new())
                .unwrap()
                .amount
                .minor_units(),
            2550
        );
        e.outflow_text = "1".into();
        assert!(
            e.build_transaction(BudgetId::new())
                .unwrap_err()
                .amount
                .is_some()
        );
        let mut missing = editor();
        missing.account_id = None;
        missing.category_id = None;
        let errors = missing.build_transaction(BudgetId::new()).unwrap_err();
        assert!(errors.account.is_some() && errors.category.is_some());
    }
    fn split_editor(parent: &str, amounts: &[&str]) -> TransactionEditorState {
        let mut e = editor();
        e.category_id = None;
        e.inflow_text = parent.into();
        e.splits = amounts
            .iter()
            .map(|x| SplitLineForm {
                category_id: Some(CategoryId::new()),
                amount_text: (*x).into(),
                memo: String::new(),
            })
            .collect();
        e
    }
    #[test]
    fn splits_enforce_shape_categories_exact_cents_and_overflow() {
        assert!(
            split_editor("1", &["1"])
                .build_transaction(BudgetId::new())
                .unwrap_err()
                .form
                .unwrap()
                .contains("two")
        );
        let mut missing = split_editor("1", &["0.5", "0.5"]);
        missing.splits[1].category_id = None;
        assert!(
            missing
                .build_transaction(BudgetId::new())
                .unwrap_err()
                .split_lines[1]
                .is_some()
        );
        let exact = split_editor("1", &["0.5", "0.5"]).build_transaction(BudgetId::new());
        assert!(exact.is_ok(), "{exact:?}");
        assert!(
            split_editor("1", &["0.5", "0.49"])
                .build_transaction(BudgetId::new())
                .unwrap_err()
                .form
                .is_some()
        );
        assert!(
            split_editor("92233720368547758.07", &["92233720368547758.07", "0.01"])
                .build_transaction(BudgetId::new())
                .unwrap_err()
                .form
                .is_some()
        );
        let mut lines = vec![
            Subtransaction {
                category_id: CategoryId::new(),
                amount: Money::from_minor_units(33),
                memo: None,
            },
            Subtransaction {
                category_id: CategoryId::new(),
                amount: Money::from_minor_units(33),
                memo: None,
            },
        ];
        TransactionBody::distribute_remainder(Money::from_minor_units(100), &mut lines).unwrap();
        assert_eq!(lines[1].amount.minor_units(), 67);
        let negative = split_editor("", &["-0.75", "0.75"])
            .build_transaction(BudgetId::new())
            .unwrap();
        assert_eq!(negative.amount, Money::ZERO);
    }

    #[test]
    fn split_actions_keep_minimum_shape_and_apply_exact_final_remainder() {
        let mut e = editor();
        e.category_id = Some(CategoryId::new());
        e.inflow_text = "1.00".into();
        e.add_split();
        assert_eq!(e.splits.len(), 2);
        for line in &mut e.splits {
            line.category_id = Some(CategoryId::new());
            line.amount_text = "0.33".into();
        }
        e.distribute_remainder().unwrap();
        assert_eq!(
            parse_split_currency_field(&e.splits[1].amount_text).unwrap(),
            Money::from_minor_units(67)
        );
        e.remove_split(0);
        assert!(
            e.build_transaction(BudgetId::new())
                .unwrap_err()
                .form
                .unwrap()
                .contains("two")
        );
    }
    #[test]
    fn original_properties_survive_and_transfers_are_rejected() {
        let budget = BudgetId::new();
        let mut e = editor();
        let original = e.build_transaction(budget).unwrap();
        let mut original = Transaction {
            archived: true,
            voided: true,
            ..original
        };
        e.transaction_id = Some(original.id);
        e.original = Some(original.clone());
        e.memo = "changed".into();
        let saved = e.build_transaction(budget).unwrap();
        assert_eq!(
            (saved.id, saved.archived, saved.voided),
            (original.id, true, true)
        );
        original.body = TransactionBody::Transfer {
            transfer_id: TransferId::new(),
            source_account_id: AccountId::new(),
            destination_account_id: AccountId::new(),
            amount: Money::ZERO,
            other_account_id: AccountId::new(),
            other_amount: Money::ZERO,
            category_id: None,
            category_effect_account_id: None,
        };
        e.original = Some(original);
        assert!(e.build_transaction(budget).unwrap_err().form.is_some());
    }
    #[test]
    fn protected_confirmation_can_be_cancelled() {
        let mut e = editor();
        e.reconciled = true;
        e.closed_account = true;
        let errors = e.build_transaction(BudgetId::new()).unwrap_err();
        assert!(errors.protected_edit.is_some() && errors.closed_account.is_some());
        e.protected_edit_confirmed = true;
        e.closed_account_confirmed = true;
        assert!(e.build_transaction(BudgetId::new()).is_ok());
        e.cancel_protected_confirmations();
        assert!(!e.protected_edit_confirmed && !e.closed_account_confirmed);
    }
    #[test]
    fn blur_normalizes_only_valid_date_and_amount_text() {
        let mut e = editor();
        e.date_text = "2026-08-09".into();
        assert!(e.normalize_date_on_blur());
        assert_eq!(e.date_text, "08/09/2026");
        e.date_text = "not a date".into();
        assert!(!e.normalize_date_on_blur());
        assert_eq!(e.date_text, "not a date");
        e.outflow_text = "1,234.5".into();
        assert!(e.normalize_amount_on_blur(true));
        assert_eq!(e.outflow_text, "$1,234.50");
        e.outflow_text = "bad".into();
        assert!(!e.normalize_amount_on_blur(true));
        assert_eq!(e.outflow_text, "bad");
    }
    #[test]
    fn focus_order_and_submission_lock_are_deterministic() {
        let mut e = editor();
        e.focus_field = TransactionEditorField::Payee;
        e.move_focus(false);
        assert_eq!(e.focus_field, TransactionEditorField::Category);
        e.move_focus(true);
        assert_eq!(e.focus_field, TransactionEditorField::Payee);
        assert!(e.mutations_enabled());
        e.metadata.begin_submission(7);
        assert!(!e.mutations_enabled());
    }
}
