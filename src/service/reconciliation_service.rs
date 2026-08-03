//! Atomic reconciliation commands over the persistence-independent ledger.
use super::account_service::{Ledger, ReconciliationChange};
use crate::domain::*;
use std::collections::{HashMap, HashSet};
use thiserror::Error;
use time::OffsetDateTime;

pub const ADJUSTMENT_PAYEE: &str = "Reconciliation Balance Adjustment";

#[derive(Clone, Debug)]
pub struct ReconciliationSession {
    pub reconciliation: Reconciliation,
    /// Eligible rows as read at session start; these are optimistic concurrency tokens.
    original: HashMap<TransactionId, Transaction>,
    selected: HashSet<TransactionId>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdjustmentPreview {
    pub amount: Money,
    pub resulting_difference: Money,
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ReconciliationServiceError {
    #[error("account not found")]
    AccountNotFound,
    #[error("transaction is not eligible for this reconciliation")]
    IneligibleTransaction,
    #[error("a selected transaction is stale or was concurrently modified")]
    StaleTransaction,
    #[error("reconciliation difference must be exactly zero")]
    NonZeroDifference,
    #[error("confirmation required")]
    ConfirmationRequired,
    #[error("adjustment preview no longer matches the session")]
    StalePreview,
    #[error("reconciliation arithmetic overflow")]
    Overflow,
    #[error("transaction not found")]
    TransactionNotFound,
}

pub struct ReconciliationService<'a> {
    ledger: &'a mut Ledger,
}

impl<'a> ReconciliationService<'a> {
    pub fn new(ledger: &'a mut Ledger) -> Self {
        Self { ledger }
    }

    pub fn start(
        &self,
        account_id: AccountId,
        statement_date: StatementDate,
        ending_balance: Money,
    ) -> Result<ReconciliationSession, ReconciliationServiceError> {
        let account = self
            .ledger
            .accounts
            .get(&account_id)
            .ok_or(ReconciliationServiceError::AccountNotFound)?;
        let original: HashMap<_, _> = self
            .eligible(account_id, statement_date)
            .into_iter()
            .map(|t| (t.id, t.clone()))
            .collect();
        let selected = original
            .values()
            .filter(|t| t.clearance == Clearance::Cleared)
            .map(|t| t.id)
            .collect();
        let now = OffsetDateTime::now_utc();
        let mut session = ReconciliationSession {
            reconciliation: Reconciliation {
                id: ReconciliationId::new(),
                budget_id: account.budget_id,
                account_id,
                statement_date,
                ending_balance,
                calculated_cleared_balance: Money::ZERO,
                difference: Money::ZERO,
                included_transaction_ids: vec![],
                state: ReconciliationState::Active,
                created_at: now,
                completed_at: None,
                invalidated_at: None,
            },
            original,
            selected,
        };
        self.recalculate(&mut session)?;
        Ok(session)
    }

    pub fn eligible(&self, account_id: AccountId, through: StatementDate) -> Vec<&Transaction> {
        let mut rows: Vec<_> = self
            .ledger
            .transactions
            .values()
            .filter(|t| {
                t.account_id == account_id
                    && t.date.0 <= through.0
                    && t.clearance != Clearance::Reconciled
                    && !t.archived
                    && !t.voided
            })
            .collect();
        rows.sort_by_key(|t| (t.date.0, t.id));
        rows
    }

    fn calculated(
        &self,
        session: &ReconciliationSession,
    ) -> Result<Money, ReconciliationServiceError> {
        self.ledger
            .transactions
            .values()
            .filter(|t| {
                t.account_id == session.reconciliation.account_id
                    && t.date.0 <= session.reconciliation.statement_date.0
                    && !t.archived
                    && !t.voided
                    && (t.clearance == Clearance::Reconciled || session.selected.contains(&t.id))
            })
            .try_fold(Money::ZERO, |sum, t| sum.checked_add(t.amount))
            .map_err(|_| ReconciliationServiceError::Overflow)
    }
    fn recalculate(
        &self,
        session: &mut ReconciliationSession,
    ) -> Result<(), ReconciliationServiceError> {
        let balance = self.calculated(session)?;
        session.reconciliation.calculated_cleared_balance = balance;
        session.reconciliation.difference =
            reconciliation_difference(session.reconciliation.ending_balance, balance)
                .map_err(|_| ReconciliationServiceError::Overflow)?;
        session.reconciliation.included_transaction_ids =
            session.selected.iter().copied().collect();
        session.reconciliation.included_transaction_ids.sort();
        Ok(())
    }
    pub fn set_cleared(
        &self,
        session: &mut ReconciliationSession,
        id: TransactionId,
        cleared: bool,
    ) -> Result<(), ReconciliationServiceError> {
        if !session.original.contains_key(&id) {
            return Err(ReconciliationServiceError::IneligibleTransaction);
        }
        if cleared {
            session.selected.insert(id);
        } else {
            session.selected.remove(&id);
        }
        self.recalculate(session)
    }
    pub fn correction(
        &mut self,
        session: &mut ReconciliationSession,
        transaction: Transaction,
    ) -> Result<(), ReconciliationServiceError> {
        if !session.original.contains_key(&transaction.id)
            || transaction.account_id != session.reconciliation.account_id
            || transaction.date.0 > session.reconciliation.statement_date.0
        {
            return Err(ReconciliationServiceError::IneligibleTransaction);
        }
        self.ledger
            .transactions
            .insert(transaction.id, transaction.clone());
        session.original.insert(transaction.id, transaction);
        self.recalculate(session)
    }
    pub fn preview_adjustment(
        &self,
        session: &ReconciliationSession,
    ) -> Result<AdjustmentPreview, ReconciliationServiceError> {
        let amount = session.reconciliation.difference;
        let resulting = session
            .reconciliation
            .difference
            .checked_sub(amount)
            .map_err(|_| ReconciliationServiceError::Overflow)?;
        Ok(AdjustmentPreview {
            amount,
            resulting_difference: resulting,
        })
    }
    pub fn confirm_adjustment(
        &mut self,
        session: &mut ReconciliationSession,
        preview: &AdjustmentPreview,
        confirmed: bool,
    ) -> Result<TransactionId, ReconciliationServiceError> {
        if !confirmed {
            return Err(ReconciliationServiceError::ConfirmationRequired);
        }
        if &self.preview_adjustment(session)? != preview {
            return Err(ReconciliationServiceError::StalePreview);
        }
        let account = self
            .ledger
            .accounts
            .get(&session.reconciliation.account_id)
            .ok_or(ReconciliationServiceError::AccountNotFound)?;
        let payee_id = self
            .ledger
            .payees
            .values()
            .find(|p| p.budget_id == account.budget_id && p.name == ADJUSTMENT_PAYEE)
            .map(|p| p.id)
            .unwrap_or_else(|| {
                let p = Payee::new(account.budget_id, ADJUSTMENT_PAYEE);
                let id = p.id;
                self.ledger.payees.insert(id, p);
                id
            });
        let transaction = Transaction {
            id: TransactionId::new(),
            budget_id: account.budget_id,
            account_id: account.id,
            date: TransactionDate(session.reconciliation.statement_date.0),
            payee_id: Some(payee_id),
            amount: preview.amount,
            memo: Some("Reconciliation adjustment (Ready to Assign)".into()),
            clearance: Clearance::Cleared,
            approval: Approval::Approved,
            body: TransactionBody::OpeningBalance { category_id: None },
            archived: false,
            voided: false,
        };
        let id = transaction.id;
        self.ledger.transactions.insert(id, transaction.clone());
        session.original.insert(id, transaction);
        session.selected.insert(id);
        self.recalculate(session)?;
        Ok(id)
    }
    /// Validate and stage every write against a clone; assignment is the atomic commit point.
    pub fn complete(
        &mut self,
        session: ReconciliationSession,
    ) -> Result<Reconciliation, ReconciliationServiceError> {
        if session.reconciliation.difference != Money::ZERO {
            return Err(ReconciliationServiceError::NonZeroDifference);
        }
        for id in &session.selected {
            let current = self
                .ledger
                .transactions
                .get(id)
                .ok_or(ReconciliationServiceError::StaleTransaction)?;
            if session.original.get(id) != Some(current)
                || current.account_id != session.reconciliation.account_id
                || current.clearance == Clearance::Reconciled
            {
                return Err(ReconciliationServiceError::StaleTransaction);
            }
        }
        let calculated = self.calculated(&session)?;
        let mut staged = self.ledger.clone();
        let mut record = session.reconciliation;
        let now = OffsetDateTime::now_utc();
        record.state = ReconciliationState::Completed;
        record.completed_at = Some(now);
        record.calculated_cleared_balance = calculated;
        record.difference =
            reconciliation_difference(record.ending_balance, record.calculated_cleared_balance)
                .map_err(|_| ReconciliationServiceError::Overflow)?;
        for id in &record.included_transaction_ids {
            staged
                .transactions
                .get_mut(id)
                .ok_or(ReconciliationServiceError::StaleTransaction)?
                .clearance = Clearance::Reconciled;
        }
        staged.reconciliations.insert(record.id, record.clone());
        staged
            .audit
            .push(format!("complete reconciliation {}", record.id));
        *self.ledger = staged;
        Ok(record)
    }

    pub fn affected_reconciliations(&self, transaction_id: TransactionId) -> Vec<&Reconciliation> {
        let Some(t) = self.ledger.transactions.get(&transaction_id) else {
            return vec![];
        };
        let mut rows: Vec<_> = self
            .ledger
            .reconciliations
            .values()
            .filter(|r| {
                r.account_id == t.account_id && r.included_transaction_ids.contains(&transaction_id)
            })
            .collect();
        rows.sort_by_key(|r| r.statement_date.0);
        rows
    }

    pub fn edit_reconciled(
        &mut self,
        after: Transaction,
        confirmed: bool,
    ) -> Result<(), ReconciliationServiceError> {
        let before = self
            .ledger
            .transactions
            .get(&after.id)
            .cloned()
            .ok_or(ReconciliationServiceError::TransactionNotFound)?;
        if before.clearance == Clearance::Reconciled && !confirmed {
            return Err(ReconciliationServiceError::ConfirmationRequired);
        }
        let mut staged = self.ledger.clone();
        staged.transactions.insert(after.id, after.clone());
        invalidate_chain(
            &mut staged,
            &before,
            "update",
            Some(before.clone()),
            Some(after),
        );
        *self.ledger = staged;
        Ok(())
    }
    pub fn delete_reconciled(
        &mut self,
        id: TransactionId,
        confirmed: bool,
    ) -> Result<(), ReconciliationServiceError> {
        let before = self
            .ledger
            .transactions
            .get(&id)
            .cloned()
            .ok_or(ReconciliationServiceError::TransactionNotFound)?;
        if before.clearance == Clearance::Reconciled && !confirmed {
            return Err(ReconciliationServiceError::ConfirmationRequired);
        }
        let mut staged = self.ledger.clone();
        let ids: Vec<_> = if let TransactionBody::Transfer { transfer_id, .. } = before.body {
            staged.transactions.values().filter(|t| matches!(t.body, TransactionBody::Transfer { transfer_id: x, .. } if x == transfer_id)).map(|t| t.id).collect()
        } else {
            vec![id]
        };
        for tid in ids {
            if let Some(old) = staged.transactions.remove(&tid) {
                invalidate_chain(&mut staged, &old, "delete", Some(old.clone()), None);
            }
        }
        *self.ledger = staged;
        Ok(())
    }
}

fn invalidate_chain(
    ledger: &mut Ledger,
    transaction: &Transaction,
    operation: &str,
    before: Option<Transaction>,
    after: Option<Transaction>,
) {
    let start = ledger
        .reconciliations
        .values()
        .filter(|r| {
            r.account_id == transaction.account_id
                && r.included_transaction_ids.contains(&transaction.id)
        })
        .map(|r| r.statement_date.0)
        .min();
    if let Some(start) = start {
        let now = OffsetDateTime::now_utc();
        let ids: Vec<_> = ledger
            .reconciliations
            .values_mut()
            .filter(|r| r.account_id == transaction.account_id && r.statement_date.0 >= start)
            .map(|r| {
                r.state = ReconciliationState::PotentiallyInvalid;
                r.invalidated_at = Some(now);
                r.id
            })
            .collect();
        for reconciliation_id in ids {
            ledger.reconciliation_changes.push(ReconciliationChange {
                reconciliation_id,
                transaction_id: transaction.id,
                operation: operation.into(),
                before: before.clone(),
                after: after.clone(),
                changed_at: now,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::AccountType;
    use time::macros::date;

    fn fixture(amounts: &[i64]) -> (Ledger, AccountId) {
        let mut ledger = Ledger::default();
        let budget = BudgetId::new();
        let account = Account::new(budget, "Checking", AccountType::Checking);
        let id = account.id;
        ledger.accounts.insert(id, account);
        for amount in amounts {
            let t = Transaction {
                id: TransactionId::new(),
                budget_id: budget,
                account_id: id,
                date: TransactionDate(date!(2026 - 01 - 01)),
                payee_id: None,
                amount: Money::from_minor_units(*amount),
                memo: None,
                clearance: Clearance::Cleared,
                approval: Approval::Approved,
                body: TransactionBody::OpeningBalance { category_id: None },
                archived: false,
                voided: false,
            };
            ledger.transactions.insert(t.id, t);
        }
        (ledger, id)
    }

    #[test]
    fn exact_zero_completes_only_selected_account_rows() {
        let (mut ledger, account) = fixture(&[100, -25]);
        let other = Account::new(
            ledger.accounts[&account].budget_id,
            "Other",
            AccountType::Checking,
        );
        let other_id = other.id;
        ledger.accounts.insert(other_id, other);
        let foreign = Transaction {
            id: TransactionId::new(),
            budget_id: ledger.accounts[&account].budget_id,
            account_id: other_id,
            date: TransactionDate(date!(2026 - 01 - 01)),
            payee_id: None,
            amount: Money::from_minor_units(75),
            memo: None,
            clearance: Clearance::Cleared,
            approval: Approval::Approved,
            body: TransactionBody::OpeningBalance { category_id: None },
            archived: false,
            voided: false,
        };
        let foreign_id = foreign.id;
        ledger.transactions.insert(foreign_id, foreign);
        let mut service = ReconciliationService::new(&mut ledger);
        let session = service
            .start(
                account,
                StatementDate(date!(2026 - 01 - 31)),
                Money::from_minor_units(75),
            )
            .unwrap();
        service.complete(session).unwrap();
        assert_eq!(
            service.ledger.transactions[&foreign_id].clearance,
            Clearance::Cleared
        );
        assert!(
            service
                .ledger
                .transactions
                .values()
                .filter(|t| t.account_id == account)
                .all(|t| t.clearance == Clearance::Reconciled)
        );
    }

    #[test]
    fn stale_row_rolls_back_and_adjustment_is_visible() {
        let (mut ledger, account) = fixture(&[100]);
        let mut service = ReconciliationService::new(&mut ledger);
        let mut session = service
            .start(
                account,
                StatementDate(date!(2026 - 01 - 31)),
                Money::from_minor_units(120),
            )
            .unwrap();
        let preview = service.preview_adjustment(&session).unwrap();
        assert_eq!(preview.amount, Money::from_minor_units(20));
        let adjustment = service
            .confirm_adjustment(&mut session, &preview, true)
            .unwrap();
        assert_eq!(session.reconciliation.difference, Money::ZERO);
        assert_eq!(
            service.ledger.transactions[&adjustment].amount,
            Money::from_minor_units(20)
        );
        service
            .ledger
            .transactions
            .get_mut(&adjustment)
            .unwrap()
            .memo = Some("concurrent".into());
        let before = service.ledger.clone();
        assert_eq!(
            service.complete(session),
            Err(ReconciliationServiceError::StaleTransaction)
        );
        assert_eq!(service.ledger.transactions, before.transactions);
        assert!(service.ledger.reconciliations.is_empty());
    }

    #[test]
    fn confirmed_edit_invalidates_subsequent_history_and_audits() {
        let (mut ledger, account) = fixture(&[100]);
        let id = *ledger.transactions.keys().next().unwrap();
        let mut service = ReconciliationService::new(&mut ledger);
        let session = service
            .start(
                account,
                StatementDate(date!(2026 - 01 - 31)),
                Money::from_minor_units(100),
            )
            .unwrap();
        service.complete(session).unwrap();
        let before = service.ledger.transactions[&id].clone();
        let mut after = before.clone();
        after.memo = Some("changed".into());
        assert_eq!(
            service.edit_reconciled(after.clone(), false),
            Err(ReconciliationServiceError::ConfirmationRequired)
        );
        service.edit_reconciled(after, true).unwrap();
        assert!(
            service
                .ledger
                .reconciliations
                .values()
                .all(|r| r.state == ReconciliationState::PotentiallyInvalid)
        );
        assert_eq!(service.ledger.reconciliation_changes.len(), 1);
        assert_eq!(
            service.ledger.reconciliation_changes[0].before.as_ref(),
            Some(&before)
        );
    }
}
