use super::account_service::Ledger;
use crate::domain::*;
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

pub struct TransactionService<'a> {
    ledger: &'a mut Ledger,
}
impl<'a> TransactionService<'a> {
    pub fn new(ledger: &'a mut Ledger) -> Self {
        Self { ledger }
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
        if matches!(
            self.ledger.transactions[&id].body,
            TransactionBody::Transfer { .. }
        ) {
            return Err(TransactionServiceError::ConfirmationRequired);
        }
        self.ledger.transactions.remove(&id);
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
        *self.ledger = staged;
        Ok(result)
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
        Ok(())
    }
    pub fn delete_transfer_both(
        &mut self,
        id: TransactionId,
        confirmed: bool,
    ) -> Result<(), TransactionServiceError> {
        let pair = self.pair(id)?;
        self.mutation_check(id, confirmed)?;
        self.mutation_check(pair, confirmed)?;
        self.ledger.transactions.remove(&id);
        self.ledger.transactions.remove(&pair);
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
