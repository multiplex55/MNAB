use crate::domain::*;
use std::collections::HashMap;
use thiserror::Error;

#[derive(Clone, Debug, Default)]
pub struct Ledger {
    pub accounts: HashMap<AccountId, Account>,
    pub transactions: HashMap<TransactionId, Transaction>,
    pub payees: HashMap<PayeeId, Payee>,
    pub audit: Vec<String>,
    pub hide_closed: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BalanceResolution {
    TransferTo(AccountId),
    ExplicitAdjustment,
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum AccountServiceError {
    #[error("account not found")]
    NotFound,
    #[error("account has transactions")]
    InUse,
    #[error("account classification cannot change after use")]
    ClassificationLocked,
    #[error("a non-zero account requires a closing balance resolution")]
    BalanceResolutionRequired,
    #[error("invalid balance resolution")]
    InvalidResolution,
    #[error("money overflow")]
    Overflow,
}

pub struct AccountService<'a> {
    ledger: &'a mut Ledger,
}
impl<'a> AccountService<'a> {
    pub fn new(ledger: &'a mut Ledger) -> Self {
        Self { ledger }
    }
    /// Account and optional visible opening transaction are staged and committed together.
    pub fn create(
        &mut self,
        budget_id: BudgetId,
        name: impl Into<String>,
        account_type: AccountType,
        opening_magnitude: Money,
        date: TransactionDate,
    ) -> Result<Account, AccountServiceError> {
        let mut staged = self.ledger.clone();
        let account = Account::new(budget_id, name, account_type);
        let amount = account_type
            .opening_amount(opening_magnitude)
            .map_err(|_| AccountServiceError::Overflow)?;
        staged.accounts.insert(account.id, account.clone());
        if amount != Money::ZERO {
            let transaction = Transaction {
                id: TransactionId::new(),
                budget_id,
                account_id: account.id,
                date,
                payee_id: None,
                amount,
                memo: Some("Opening Balance".into()),
                clearance: Clearance::Cleared,
                approval: Approval::Approved,
                body: TransactionBody::OpeningBalance { category_id: None },
                archived: false,
                voided: false,
            };
            staged.transactions.insert(transaction.id, transaction);
        }
        *self.ledger = staged;
        Ok(account)
    }
    fn account_mut(&mut self, id: AccountId) -> Result<&mut Account, AccountServiceError> {
        self.ledger
            .accounts
            .get_mut(&id)
            .ok_or(AccountServiceError::NotFound)
    }
    pub fn rename(
        &mut self,
        id: AccountId,
        name: impl Into<String>,
    ) -> Result<(), AccountServiceError> {
        self.account_mut(id)?.name = name.into();
        Ok(())
    }
    pub fn annotate(
        &mut self,
        id: AccountId,
        note: Option<String>,
    ) -> Result<(), AccountServiceError> {
        self.account_mut(id)?.note = note;
        Ok(())
    }
    pub fn reorder(&mut self, id: AccountId, order: i64) -> Result<(), AccountServiceError> {
        self.account_mut(id)?.sort_order = order;
        Ok(())
    }
    pub fn favorite(&mut self, id: AccountId, favorite: bool) -> Result<(), AccountServiceError> {
        self.account_mut(id)?.favorite = favorite;
        Ok(())
    }
    pub fn hide_closed(&mut self, hide: bool) {
        self.ledger.hide_closed = hide;
    }
    pub fn reopen(&mut self, id: AccountId) -> Result<(), AccountServiceError> {
        self.account_mut(id)?.closed = false;
        Ok(())
    }
    pub fn change_type(
        &mut self,
        id: AccountId,
        new_type: AccountType,
    ) -> Result<(), AccountServiceError> {
        let old = self
            .ledger
            .accounts
            .get(&id)
            .ok_or(AccountServiceError::NotFound)?
            .account_type;
        if old.classification() != new_type.classification()
            && self
                .ledger
                .transactions
                .values()
                .any(|t| t.account_id == id)
        {
            return Err(AccountServiceError::ClassificationLocked);
        }
        self.account_mut(id)?.account_type = new_type;
        Ok(())
    }
    pub fn close(
        &mut self,
        id: AccountId,
        resolution: Option<BalanceResolution>,
        date: TransactionDate,
    ) -> Result<(), AccountServiceError> {
        let balance = crate::calculation::balances::derive(
            self.ledger
                .transactions
                .values()
                .filter(|t| t.account_id == id),
        )
        .map_err(|_| AccountServiceError::Overflow)?
        .working;
        if balance != Money::ZERO && resolution.is_none() {
            return Err(AccountServiceError::BalanceResolutionRequired);
        }
        let mut staged = self.ledger.clone();
        if balance != Money::ZERO {
            let amount = balance
                .checked_neg()
                .map_err(|_| AccountServiceError::Overflow)?;
            match resolution.unwrap() {
                BalanceResolution::ExplicitAdjustment => {
                    let a = staged
                        .accounts
                        .get(&id)
                        .ok_or(AccountServiceError::NotFound)?;
                    let t = Transaction {
                        id: TransactionId::new(),
                        budget_id: a.budget_id,
                        account_id: id,
                        date,
                        payee_id: None,
                        amount,
                        memo: Some("Closing balance adjustment".into()),
                        clearance: Clearance::Cleared,
                        approval: Approval::Approved,
                        body: TransactionBody::OpeningBalance { category_id: None },
                        archived: false,
                        voided: false,
                    };
                    staged.transactions.insert(t.id, t);
                }
                BalanceResolution::TransferTo(other) => {
                    crate::service::transaction_service::create_transfer_in(
                        &mut staged,
                        id,
                        other,
                        date,
                        amount,
                    )
                    .map_err(|_| AccountServiceError::InvalidResolution)?;
                }
            }
        }
        staged
            .accounts
            .get_mut(&id)
            .ok_or(AccountServiceError::NotFound)?
            .closed = true;
        *self.ledger = staged;
        Ok(())
    }
    pub fn delete_if_unused(&mut self, id: AccountId) -> Result<(), AccountServiceError> {
        if self
            .ledger
            .transactions
            .values()
            .any(|t| t.account_id == id)
        {
            return Err(AccountServiceError::InUse);
        }
        self.ledger
            .accounts
            .remove(&id)
            .ok_or(AccountServiceError::NotFound)?;
        Ok(())
    }
}
