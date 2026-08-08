use super::account_service::Ledger;
use crate::domain::*;
use crate::{
    app::command::{TransactionBatchAction, TransactionBatchCommand, TransactionBatchSelection},
    error::RepositoryError,
    storage::repository::{AccountRepository, AuditRepository, TransactionRepository},
};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

#[derive(Debug, Error, Eq, PartialEq)]
pub enum TransactionServiceError {
    #[error("transaction not found")]
    NotFound,
    #[error("account not found")]
    AccountNotFound,
    #[error("confirmation required")]
    ConfirmationRequired,
    #[error("invalid transaction: {0}")]
    Invalid(TransactionError),
    #[error("invalid transfer")]
    InvalidTransfer,
}

pub const MAX_BATCH_TRANSACTIONS: usize = 100_000;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum BatchPreflightReason {
    Eligible,
    Reconciled,
    ClosedAccount,
    Transfer,
    Split,
    ArchivedOrVoided,
    MissingOrChanged,
    Incompatible,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BatchPreflightResult {
    pub counts: BTreeMap<BatchPreflightReason, usize>,
    pub transaction_ids: BTreeMap<BatchPreflightReason, Vec<TransactionId>>,
    pub rejection: Option<&'static str>,
}
impl BatchPreflightResult {
    fn add(&mut self, reason: BatchPreflightReason, id: TransactionId) {
        *self.counts.entry(reason).or_default() += 1;
        self.transaction_ids.entry(reason).or_default().push(id);
    }
    pub fn count(&self, reason: BatchPreflightReason) -> usize {
        self.counts.get(&reason).copied().unwrap_or(0)
    }
    pub fn is_allowed(&self) -> bool {
        self.rejection.is_none()
            && self.count(BatchPreflightReason::Incompatible) == 0
            && self.count(BatchPreflightReason::MissingOrChanged) == 0
            && self.count(BatchPreflightReason::ClosedAccount) == 0
    }
}

/// Classifies the complete snapshot before the first write. Callers must invoke this after opening
/// their unit of work; `execute_batch` deliberately repeats it to close the TOCTOU window.
pub fn batch_preflight<R: TransactionRepository + AccountRepository>(
    r: &mut R,
    command: &TransactionBatchCommand,
) -> Result<(BatchPreflightResult, Vec<Transaction>), RepositoryError> {
    let rows = r.selected_transactions(&command.selection, MAX_BATCH_TRANSACTIONS)?;
    let mut result = BatchPreflightResult {
        counts: BTreeMap::new(),
        transaction_ids: BTreeMap::new(),
        rejection: None,
    };
    if let TransactionBatchSelection::Explicit(ids) = &command.selection {
        let found: BTreeSet<_> = rows.iter().map(|t| t.id).collect();
        for id in ids.difference(&found) {
            result.add(BatchPreflightReason::MissingOrChanged, *id);
        }
    }
    for t in &rows {
        if t.clearance == Clearance::Reconciled {
            result.add(BatchPreflightReason::Reconciled, t.id);
        }
        if r.account(t.account_id)?.is_none_or(|a| a.closed) {
            result.add(BatchPreflightReason::ClosedAccount, t.id);
        }
        if t.archived || t.voided {
            result.add(BatchPreflightReason::ArchivedOrVoided, t.id);
        }
        match t.body {
            TransactionBody::Transfer { .. } => result.add(BatchPreflightReason::Transfer, t.id),
            TransactionBody::Split { .. } => result.add(BatchPreflightReason::Split, t.id),
            _ => {}
        }
        let incompatible = matches!(
            (&command.action, &t.body),
            (
                TransactionBatchAction::SetCategory(_),
                TransactionBody::Transfer { .. } | TransactionBody::Split { .. }
            )
        ) || matches!(
            command.action,
            TransactionBatchAction::Void | TransactionBatchAction::Delete
        ) && (t.archived || t.voided);
        if incompatible {
            result.add(BatchPreflightReason::Incompatible, t.id);
        } else {
            result.add(BatchPreflightReason::Eligible, t.id);
        }
    }
    if result.count(BatchPreflightReason::Reconciled) > 0 {
        result.rejection = Some(
            "Reconciled transactions cannot be changed in bulk; deselect them or edit them individually.",
        );
    }
    Ok((result, rows))
}

pub fn execute_batch<R: TransactionRepository + AccountRepository + AuditRepository>(
    r: &mut R,
    command: &TransactionBatchCommand,
) -> Result<(BatchPreflightResult, Vec<Transaction>, Vec<TransactionId>), RepositoryError> {
    if let TransactionBatchAction::Restore(rows) = &command.action {
        let mut prior = Vec::new();
        let mut affected = Vec::new();
        for t in rows {
            if let Some(old) = r.transaction(t.id)? {
                prior.push(old);
            }
            t.validate().map_err(|_| RepositoryError::Failed {
                source: Box::new(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "invalid inverse transaction",
                )),
            })?;
            r.put_transaction(t)?;
            r.append_audit("transaction", &t.id.to_string(), "batch restore")?;
            affected.push(t.id);
        }
        let mut p = BatchPreflightResult {
            counts: BTreeMap::new(),
            transaction_ids: BTreeMap::new(),
            rejection: None,
        };
        for id in &affected {
            p.add(BatchPreflightReason::Eligible, *id)
        }
        return Ok((p, prior, affected));
    }
    let (preflight, rows) = batch_preflight(r, command)?;
    if !preflight.is_allowed() {
        return Err(RepositoryError::Failed {
            source: Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                preflight
                    .rejection
                    .unwrap_or("bulk transaction preflight rejected"),
            )),
        });
    }
    let mut affected = BTreeSet::new();
    let mut prior = Vec::new();
    for row in rows {
        let mut group = vec![row.clone()];
        if let TransactionBody::Transfer { transfer_id, .. } = row.body {
            let all = r.selected_transactions(
                &TransactionBatchSelection::AllMatching {
                    query: crate::app::register::CanonicalQuery {
                        scope: crate::app::view_model::RegisterScope::AllTransactions,
                        filter: Default::default(),
                        sort_field: crate::app::view_model::RegisterSortField::Date,
                        sort_direction: crate::app::view_model::RegisterSortDirection::Ascending,
                        revision: 0,
                    },
                    exclusions: BTreeSet::new(),
                },
                MAX_BATCH_TRANSACTIONS,
            )?;
            group=all.into_iter().filter(|t|matches!(t.body,TransactionBody::Transfer{transfer_id:id,..} if id==transfer_id)).collect();
            if group.len() != 2 {
                return Err(RepositoryError::Failed {
                    source: Box::new(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "transfer pair is incomplete",
                    )),
                });
            }
        }
        for mut t in group {
            if !affected.insert(t.id) {
                continue;
            }
            prior.push(t.clone());
            match &command.action {
                TransactionBatchAction::SetApproval(v) => t.approval = *v,
                TransactionBatchAction::SetCategory(id) => {
                    t.body = TransactionBody::categorized(*id)
                }
                TransactionBatchAction::SetPayee(id) => t.payee_id = *id,
                TransactionBatchAction::SetClearance(v) => t.clearance = *v,
                TransactionBatchAction::SetMemo(v) => t.memo = v.clone(),
                TransactionBatchAction::Void => t.voided = true,
                TransactionBatchAction::Delete => {
                    r.delete_transaction(t.id)?;
                    r.append_audit("transaction", &t.id.to_string(), "batch delete")?;
                    continue;
                }
                TransactionBatchAction::Restore(_) => unreachable!(),
            }
            t.validate().map_err(|_| RepositoryError::Failed {
                source: Box::new(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "batch produced invalid transaction",
                )),
            })?;
            r.put_transaction(&t)?;
            r.append_audit("transaction", &t.id.to_string(), "batch update")?;
        }
    }
    Ok((preflight, prior, affected.into_iter().collect()))
}

pub struct TransactionService<'a> {
    ledger: &'a mut Ledger,
}
impl<'a> TransactionService<'a> {
    pub fn new(ledger: &'a mut Ledger) -> Self {
        Self { ledger }
    }
    fn invalidate_from(&mut self, date: TransactionDate) {
        self.ledger.recalculation_from = Some(
            self.ledger
                .recalculation_from
                .map_or(date, |old| if old.0 <= date.0 { old } else { date }),
        );
    }
    fn check(
        &self,
        transaction: &Transaction,
        confirmed: bool,
    ) -> Result<(), TransactionServiceError> {
        let account = self
            .ledger
            .accounts
            .get(&transaction.account_id)
            .ok_or(TransactionServiceError::AccountNotFound)?;
        if account.closed && !confirmed {
            return Err(TransactionServiceError::ConfirmationRequired);
        }
        transaction
            .validate()
            .map_err(TransactionServiceError::Invalid)
    }
    fn mutation_check(
        &self,
        id: TransactionId,
        confirmed: bool,
    ) -> Result<(), TransactionServiceError> {
        let transaction = self
            .ledger
            .transactions
            .get(&id)
            .ok_or(TransactionServiceError::NotFound)?;
        if transaction.clearance == Clearance::Reconciled && !confirmed {
            Err(TransactionServiceError::ConfirmationRequired)
        } else {
            Ok(())
        }
    }
    pub fn add(
        &mut self,
        transaction: Transaction,
        confirmed: bool,
    ) -> Result<(), TransactionServiceError> {
        self.check(&transaction, confirmed)?;
        self.invalidate_from(transaction.date);
        self.ledger.transactions.insert(transaction.id, transaction);
        self.ledger.audit.push("add transaction".into());
        Ok(())
    }
    pub fn edit(
        &mut self,
        transaction: Transaction,
        confirmed: bool,
    ) -> Result<(), TransactionServiceError> {
        self.mutation_check(transaction.id, confirmed)?;
        let old_date = self.ledger.transactions[&transaction.id].date;
        self.check(&transaction, confirmed)?;
        if matches!(
            self.ledger.transactions[&transaction.id].body,
            TransactionBody::Transfer { .. }
        ) {
            return self.edit_transfer(
                transaction.id,
                transaction.date,
                transaction.amount,
                confirmed,
            );
        }
        self.ledger.transactions.insert(transaction.id, transaction);
        self.invalidate_from(old_date);
        self.ledger
            .audit
            .push(if confirmed { "confirmed edit" } else { "edit" }.into());
        Ok(())
    }
    pub fn delete(
        &mut self,
        id: TransactionId,
        confirmed: bool,
    ) -> Result<(), TransactionServiceError> {
        self.mutation_check(id, confirmed)?;
        let date = self.ledger.transactions[&id].date;
        if matches!(
            self.ledger.transactions[&id].body,
            TransactionBody::Transfer { .. }
        ) {
            return Err(TransactionServiceError::ConfirmationRequired);
        }
        self.ledger.transactions.remove(&id);
        self.invalidate_from(date);
        self.ledger.audit.push("delete".into());
        Ok(())
    }
    pub fn duplicate(
        &mut self,
        id: TransactionId,
    ) -> Result<TransactionId, TransactionServiceError> {
        let mut t = self
            .ledger
            .transactions
            .get(&id)
            .cloned()
            .ok_or(TransactionServiceError::NotFound)?;
        t.id = TransactionId::new();
        t.clearance = Clearance::Uncleared;
        t.approval = Approval::Unapproved;
        if matches!(t.body, TransactionBody::Transfer { .. }) {
            return Err(TransactionServiceError::InvalidTransfer);
        }
        self.add(t.clone(), false)?;
        Ok(t.id)
    }
    pub fn approve(&mut self, id: TransactionId) -> Result<(), TransactionServiceError> {
        self.ledger
            .transactions
            .get_mut(&id)
            .ok_or(TransactionServiceError::NotFound)?
            .approval = Approval::Approved;
        Ok(())
    }
    pub fn set_clearance(
        &mut self,
        id: TransactionId,
        state: Clearance,
        confirmed: bool,
    ) -> Result<(), TransactionServiceError> {
        self.mutation_check(id, confirmed)?;
        self.ledger.transactions.get_mut(&id).unwrap().clearance = state;
        Ok(())
    }
    pub fn categorize(
        &mut self,
        id: TransactionId,
        category_id: CategoryId,
        confirmed: bool,
    ) -> Result<(), TransactionServiceError> {
        self.mutation_check(id, confirmed)?;
        let t = self.ledger.transactions.get_mut(&id).unwrap();
        if matches!(t.body, TransactionBody::Transfer { .. }) {
            return Err(TransactionServiceError::InvalidTransfer);
        }
        t.body = TransactionBody::categorized(category_id);
        Ok(())
    }
    pub fn change_payee(
        &mut self,
        id: TransactionId,
        payee_id: Option<PayeeId>,
        confirmed: bool,
    ) -> Result<(), TransactionServiceError> {
        self.mutation_check(id, confirmed)?;
        self.ledger.transactions.get_mut(&id).unwrap().payee_id = payee_id;
        Ok(())
    }
    pub fn move_to_account(
        &mut self,
        id: TransactionId,
        account_id: AccountId,
        confirmed: bool,
    ) -> Result<(), TransactionServiceError> {
        self.mutation_check(id, confirmed)?;
        let account = self
            .ledger
            .accounts
            .get(&account_id)
            .ok_or(TransactionServiceError::AccountNotFound)?;
        if account.closed && !confirmed {
            return Err(TransactionServiceError::ConfirmationRequired);
        }
        let t = self.ledger.transactions.get_mut(&id).unwrap();
        if matches!(t.body, TransactionBody::Transfer { .. }) {
            return Err(TransactionServiceError::InvalidTransfer);
        }
        t.account_id = account_id;
        Ok(())
    }
    pub fn batch_approve(&mut self, ids: &[TransactionId]) -> Result<(), TransactionServiceError> {
        self.atomic(|service| {
            for id in ids {
                service.approve(*id)?;
            }
            Ok(())
        })
    }
    pub fn batch_delete(
        &mut self,
        ids: &[TransactionId],
        confirmed: bool,
    ) -> Result<(), TransactionServiceError> {
        self.atomic(|service| {
            for id in ids {
                service.delete(*id, confirmed)?;
            }
            Ok(())
        })
    }
    fn atomic(
        &mut self,
        operation: impl FnOnce(&mut Self) -> Result<(), TransactionServiceError>,
    ) -> Result<(), TransactionServiceError> {
        let before = self.ledger.clone();
        if let Err(error) = operation(self) {
            *self.ledger = before;
            Err(error)
        } else {
            self.ledger.audit.push("atomic batch".into());
            Ok(())
        }
    }
    pub fn create_transfer(
        &mut self,
        source: AccountId,
        destination: AccountId,
        date: TransactionDate,
        source_amount: Money,
    ) -> Result<(TransactionId, TransactionId), TransactionServiceError> {
        let mut staged = self.ledger.clone();
        let result = create_transfer_in(&mut staged, source, destination, date, source_amount)?;
        staged.recalculation_from = Some(
            staged
                .recalculation_from
                .map_or(date, |old| if old.0 <= date.0 { old } else { date }),
        );
        *self.ledger = staged;
        Ok(result)
    }
    pub fn create_categorized_transfer(
        &mut self,
        source: AccountId,
        destination: AccountId,
        date: TransactionDate,
        source_amount: Money,
        category: Option<CategoryId>,
        effect: Option<AccountId>,
        memo: Option<String>,
    ) -> Result<(TransactionId, TransactionId), TransactionServiceError> {
        let mut staged = self.ledger.clone();
        let ids = create_transfer_in(&mut staged, source, destination, date, source_amount)?;
        if effect.is_some_and(|id| id != source && id != destination) {
            return Err(TransactionServiceError::InvalidTransfer);
        }
        for id in [ids.0, ids.1] {
            let transaction = staged.transactions.get_mut(&id).unwrap();
            transaction.memo = memo.clone();
            if let TransactionBody::Transfer {
                category_id,
                category_effect_account_id,
                ..
            } = &mut transaction.body
            {
                *category_id = category;
                *category_effect_account_id = effect;
            }
        }
        staged.recalculation_from = Some(
            staged
                .recalculation_from
                .map_or(date, |old| if old.0 <= date.0 { old } else { date }),
        );
        staged
            .audit
            .push("atomic categorized transfer create".into());
        *self.ledger = staged;
        Ok(ids)
    }
    fn pair(&self, id: TransactionId) -> Result<TransactionId, TransactionServiceError> {
        let t = self
            .ledger
            .transactions
            .get(&id)
            .ok_or(TransactionServiceError::NotFound)?;
        let transfer_id = match t.body {
            TransactionBody::Transfer { transfer_id, .. } => transfer_id,
            _ => return Err(TransactionServiceError::InvalidTransfer),
        };
        self.ledger.transactions.values().find(|other| other.id != id && matches!(other.body, TransactionBody::Transfer { transfer_id: x, .. } if x == transfer_id)).map(|t| t.id).ok_or(TransactionServiceError::InvalidTransfer)
    }
    pub fn edit_transfer(
        &mut self,
        id: TransactionId,
        date: TransactionDate,
        amount: Money,
        confirmed: bool,
    ) -> Result<(), TransactionServiceError> {
        let pair = self.pair(id)?;
        let old_date = self.ledger.transactions[&id].date;
        self.mutation_check(id, confirmed)?;
        self.mutation_check(pair, confirmed)?;
        let opposite = amount
            .checked_neg()
            .map_err(|_| TransactionServiceError::InvalidTransfer)?;
        self.ledger.transactions.get_mut(&id).unwrap().date = date;
        self.ledger.transactions.get_mut(&id).unwrap().amount = amount;
        if let TransactionBody::Transfer { other_amount, .. } =
            &mut self.ledger.transactions.get_mut(&id).unwrap().body
        {
            *other_amount = opposite;
        }
        self.ledger.transactions.get_mut(&pair).unwrap().date = date;
        self.ledger.transactions.get_mut(&pair).unwrap().amount = opposite;
        if let TransactionBody::Transfer { other_amount, .. } =
            &mut self.ledger.transactions.get_mut(&pair).unwrap().body
        {
            *other_amount = amount;
        }
        self.invalidate_from(if old_date.0 <= date.0 { old_date } else { date });
        Ok(())
    }
    pub fn delete_transfer_both(
        &mut self,
        id: TransactionId,
        confirmed: bool,
    ) -> Result<(), TransactionServiceError> {
        let pair = self.pair(id)?;
        let date = self.ledger.transactions[&id].date;
        let pair_date = self.ledger.transactions[&pair].date;
        let date = if date.0 <= pair_date.0 {
            date
        } else {
            pair_date
        };
        self.mutation_check(id, confirmed)?;
        self.mutation_check(pair, confirmed)?;
        self.ledger.transactions.remove(&id);
        self.ledger.transactions.remove(&pair);
        self.invalidate_from(date);
        Ok(())
    }
    /// Unlinked sides become ordinary uncategorized adjustments (represented by opening/adjustment body).
    pub fn unlink_transfer(
        &mut self,
        id: TransactionId,
        confirmed: bool,
    ) -> Result<(), TransactionServiceError> {
        let pair = self.pair(id)?;
        self.mutation_check(id, confirmed)?;
        self.mutation_check(pair, confirmed)?;
        self.ledger.transactions.get_mut(&id).unwrap().body =
            TransactionBody::OpeningBalance { category_id: None };
        self.ledger.transactions.get_mut(&pair).unwrap().body =
            TransactionBody::OpeningBalance { category_id: None };
        Ok(())
    }
}

pub(crate) fn create_transfer_in(
    ledger: &mut Ledger,
    source: AccountId,
    destination: AccountId,
    date: TransactionDate,
    source_amount: Money,
) -> Result<(TransactionId, TransactionId), TransactionServiceError> {
    let a = ledger
        .accounts
        .get(&source)
        .ok_or(TransactionServiceError::AccountNotFound)?
        .clone();
    let b = ledger
        .accounts
        .get(&destination)
        .ok_or(TransactionServiceError::AccountNotFound)?
        .clone();
    let destination_amount = source_amount
        .checked_neg()
        .map_err(|_| TransactionServiceError::InvalidTransfer)?;
    let transfer_id = TransferId::new();
    let (left_body, right_body) =
        TransactionBody::transfer(transfer_id, &a, source_amount, &b, destination_amount)
            .map_err(TransactionServiceError::Invalid)?;
    let left = Transaction {
        id: TransactionId::new(),
        budget_id: a.budget_id,
        account_id: source,
        date,
        payee_id: None,
        amount: source_amount,
        memo: None,
        clearance: Clearance::Uncleared,
        approval: Approval::Approved,
        body: left_body,
        archived: false,
        voided: false,
    };
    let right = Transaction {
        id: TransactionId::new(),
        budget_id: b.budget_id,
        account_id: destination,
        date,
        payee_id: None,
        amount: destination_amount,
        memo: None,
        clearance: Clearance::Uncleared,
        approval: Approval::Approved,
        body: right_body,
        archived: false,
        voided: false,
    };
    let result = (left.id, right.id);
    ledger.transactions.insert(left.id, left);
    ledger.transactions.insert(right.id, right);
    Ok(result)
}
