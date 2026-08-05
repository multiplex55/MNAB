use crate::domain::*;
use std::collections::HashMap;
use thiserror::Error;

#[derive(Clone, Debug, Default)]
pub struct Ledger {
    pub accounts: HashMap<AccountId, Account>,
    pub transactions: HashMap<TransactionId, Transaction>,
    pub payees: HashMap<PayeeId, Payee>,
    pub reconciliations: HashMap<ReconciliationId, Reconciliation>,
    pub reconciliation_changes: Vec<ReconciliationChange>,
    pub audit: Vec<String>,
    pub hide_closed: bool,
    pub account_groups: HashMap<AccountGroupId, AccountGroup>,
    pub category_goals: HashMap<CategoryId, CategoryGoal>,
    /// Earliest ledger date invalidated by a transaction mutation; recalculation continues forward.
    pub recalculation_from: Option<TransactionDate>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReconciliationChange {
    pub reconciliation_id: ReconciliationId,
    pub transaction_id: TransactionId,
    pub operation: String,
    pub before: Option<Transaction>,
    pub after: Option<Transaction>,
    pub changed_at: time::OffsetDateTime,
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
    #[error("opening balance must be a non-negative magnitude")]
    InvalidOpeningMagnitude,
    #[error("account group not found")]
    GroupNotFound,
    #[error("account group cannot parent itself")]
    GroupSelfParent,
    #[error("account group hierarchy would cycle")]
    GroupCycle,
    #[error("account group is not empty")]
    GroupNotEmpty,
    #[error("account and group budgets differ")]
    DifferentBudgets,
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
        if opening_magnitude < Money::ZERO
            && !(account_type == AccountType::CreditCard
                && opening_magnitude.minor_units() == i64::MIN)
        {
            return Err(AccountServiceError::InvalidOpeningMagnitude);
        }
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
        let name = name.into();
        self.account_mut(id)?.name.clone_from(&name);
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
    pub fn create_group(
        &mut self,
        budget_id: BudgetId,
        name: impl Into<String>,
        parent_group_id: Option<AccountGroupId>,
    ) -> Result<AccountGroup, AccountServiceError> {
        let mut group = AccountGroup::new(budget_id, name);
        self.validate_group_parent(group.id, budget_id, parent_group_id)?;
        group.parent_group_id = parent_group_id;
        group.sort_order = self.next_group_sort_order(budget_id, parent_group_id);
        self.ledger.account_groups.insert(group.id, group.clone());
        Ok(group)
    }

    pub fn move_group(
        &mut self,
        id: AccountGroupId,
        parent_group_id: Option<AccountGroupId>,
        sort_order: i64,
    ) -> Result<(), AccountServiceError> {
        let budget_id = self
            .ledger
            .account_groups
            .get(&id)
            .ok_or(AccountServiceError::GroupNotFound)?
            .budget_id;
        self.validate_group_parent(id, budget_id, parent_group_id)?;
        let group = self
            .ledger
            .account_groups
            .get_mut(&id)
            .ok_or(AccountServiceError::GroupNotFound)?;
        group.parent_group_id = parent_group_id;
        group.sort_order = sort_order;
        Ok(())
    }

    pub fn set_account_group(
        &mut self,
        account_id: AccountId,
        group_id: Option<AccountGroupId>,
        sort_order: i64,
    ) -> Result<(), AccountServiceError> {
        let account_budget = self
            .ledger
            .accounts
            .get(&account_id)
            .ok_or(AccountServiceError::NotFound)?
            .budget_id;
        if let Some(group_id) = group_id {
            let group = self
                .ledger
                .account_groups
                .get(&group_id)
                .ok_or(AccountServiceError::GroupNotFound)?;
            if group.budget_id != account_budget {
                return Err(AccountServiceError::DifferentBudgets);
            }
        }
        let account = self.account_mut(account_id)?;
        account.group_id = group_id;
        account.sort_order = sort_order;
        Ok(())
    }

    pub fn delete_group(
        &mut self,
        id: AccountGroupId,
        move_children_to: Option<Option<AccountGroupId>>,
    ) -> Result<(), AccountServiceError> {
        let group = self
            .ledger
            .account_groups
            .get(&id)
            .cloned()
            .ok_or(AccountServiceError::GroupNotFound)?;
        let has_children = self
            .ledger
            .account_groups
            .values()
            .any(|g| g.parent_group_id == Some(id));
        let has_accounts = self
            .ledger
            .accounts
            .values()
            .any(|a| a.group_id == Some(id));
        if (has_children || has_accounts) && move_children_to.is_none() {
            return Err(AccountServiceError::GroupNotEmpty);
        }
        if let Some(new_parent) = move_children_to {
            self.validate_group_parent(id, group.budget_id, new_parent)?;
            for child in self
                .ledger
                .account_groups
                .values_mut()
                .filter(|g| g.parent_group_id == Some(id))
            {
                child.parent_group_id = new_parent;
            }
            for account in self
                .ledger
                .accounts
                .values_mut()
                .filter(|a| a.group_id == Some(id))
            {
                account.group_id = new_parent;
            }
        }
        self.ledger.account_groups.remove(&id);
        Ok(())
    }

    #[must_use]
    pub fn ordered_group_children(
        &self,
        budget_id: BudgetId,
        parent: Option<AccountGroupId>,
    ) -> Vec<AccountGroupId> {
        let mut groups: Vec<_> = self
            .ledger
            .account_groups
            .values()
            .filter(|g| g.budget_id == budget_id && g.parent_group_id == parent)
            .collect();
        groups.sort_by_key(|g| (g.sort_order, g.name.clone(), g.id));
        groups.into_iter().map(|g| g.id).collect()
    }

    #[must_use]
    pub fn ordered_account_siblings(
        &self,
        budget_id: BudgetId,
        group_id: Option<AccountGroupId>,
    ) -> Vec<AccountId> {
        let mut accounts: Vec<_> = self
            .ledger
            .accounts
            .values()
            .filter(|a| a.budget_id == budget_id && a.group_id == group_id)
            .collect();
        accounts.sort_by_key(|a| (a.sort_order, a.name.clone(), a.id));
        accounts.into_iter().map(|a| a.id).collect()
    }

    fn next_group_sort_order(&self, budget_id: BudgetId, parent: Option<AccountGroupId>) -> i64 {
        self.ledger
            .account_groups
            .values()
            .filter(|g| g.budget_id == budget_id && g.parent_group_id == parent)
            .map(|g| g.sort_order)
            .max()
            .unwrap_or(-1)
            + 1
    }

    fn validate_group_parent(
        &self,
        id: AccountGroupId,
        budget_id: BudgetId,
        parent: Option<AccountGroupId>,
    ) -> Result<(), AccountServiceError> {
        if parent == Some(id) {
            return Err(AccountServiceError::GroupSelfParent);
        }
        let mut cursor = parent;
        while let Some(parent_id) = cursor {
            if parent_id == id {
                return Err(AccountServiceError::GroupCycle);
            }
            let parent_group = self
                .ledger
                .account_groups
                .get(&parent_id)
                .ok_or(AccountServiceError::GroupNotFound)?;
            if parent_group.budget_id != budget_id {
                return Err(AccountServiceError::DifferentBudgets);
            }
            cursor = parent_group.parent_group_id;
        }
        Ok(())
    }
}

#[cfg(test)]
mod lifecycle_tests {
    use super::*;
    use time::macros::date;

    fn day() -> TransactionDate {
        TransactionDate(date!(2026 - 08 - 04))
    }

    #[test]
    fn every_account_type_applies_the_documented_opening_sign() {
        for (kind, expected) in [
            (AccountType::Checking, 100),
            (AccountType::Savings, 100),
            (AccountType::Cash, 100),
            (AccountType::CreditCard, -100),
            (AccountType::Loan, -100),
            (AccountType::Asset, 100),
            (AccountType::Liability, -100),
        ] {
            assert_eq!(
                kind.opening_amount(Money::from_minor_units(100))
                    .unwrap()
                    .minor_units(),
                expected
            );
        }
    }

    #[test]
    fn opening_balance_is_a_visible_approved_transaction() {
        let mut ledger = Ledger::default();
        let account = AccountService::new(&mut ledger)
            .create(
                BudgetId::new(),
                "Cash",
                AccountType::Cash,
                Money::from_minor_units(500),
                day(),
            )
            .unwrap();
        let transaction = ledger
            .transactions
            .values()
            .find(|t| t.account_id == account.id)
            .unwrap();
        assert!(!transaction.archived && !transaction.voided);
        assert_eq!(transaction.memo.as_deref(), Some("Opening Balance"));
        assert!(matches!(
            transaction.body,
            TransactionBody::OpeningBalance { .. }
        ));
    }

    #[test]
    fn nonzero_close_requires_resolution_and_referenced_account_cannot_be_deleted() {
        let mut ledger = Ledger::default();
        let account = AccountService::new(&mut ledger)
            .create(
                BudgetId::new(),
                "Cash",
                AccountType::Cash,
                Money::from_minor_units(500),
                day(),
            )
            .unwrap();
        let mut service = AccountService::new(&mut ledger);
        assert_eq!(
            service.close(account.id, None, day()),
            Err(AccountServiceError::BalanceResolutionRequired)
        );
        assert_eq!(
            service.delete_if_unused(account.id),
            Err(AccountServiceError::InUse)
        );
        service
            .close(
                account.id,
                Some(BalanceResolution::ExplicitAdjustment),
                day(),
            )
            .unwrap();
    }
}

#[cfg(test)]
mod account_group_tests {
    use super::*;
    use time::macros::date;

    fn day() -> TransactionDate {
        TransactionDate(date!(2026 - 08 - 05))
    }

    #[test]
    fn group_cycle_rejection_self_direct_and_deep() {
        let mut ledger = Ledger::default();
        let budget = BudgetId::new();
        let mut service = AccountService::new(&mut ledger);
        let parent = service.create_group(budget, "Parent", None).unwrap();
        assert_eq!(
            service.move_group(parent.id, Some(parent.id), 0),
            Err(AccountServiceError::GroupSelfParent)
        );
        let child = service
            .create_group(budget, "Child", Some(parent.id))
            .unwrap();
        assert_eq!(
            service.move_group(parent.id, Some(child.id), 0),
            Err(AccountServiceError::GroupCycle)
        );
        let grandchild = service
            .create_group(budget, "Grandchild", Some(child.id))
            .unwrap();
        assert_eq!(
            service.move_group(parent.id, Some(grandchild.id), 0),
            Err(AccountServiceError::GroupCycle)
        );
    }

    #[test]
    fn moving_reordering_groups_and_accounts_yields_deterministic_order() {
        let mut ledger = Ledger::default();
        let budget = BudgetId::new();
        let mut service = AccountService::new(&mut ledger);
        let z = service.create_group(budget, "Z", None).unwrap();
        let a = service.create_group(budget, "A", None).unwrap();
        service.move_group(z.id, None, 10).unwrap();
        service.move_group(a.id, None, 10).unwrap();
        assert_eq!(
            service.ordered_group_children(budget, None),
            vec![a.id, z.id]
        );
        let cash = service
            .create(budget, "Cash", AccountType::Cash, Money::ZERO, day())
            .unwrap();
        let bank = service
            .create(budget, "Bank", AccountType::Checking, Money::ZERO, day())
            .unwrap();
        service.set_account_group(cash.id, Some(a.id), 1).unwrap();
        service.set_account_group(bank.id, Some(a.id), 1).unwrap();
        assert_eq!(
            service.ordered_account_siblings(budget, Some(a.id)),
            vec![bank.id, cash.id]
        );
    }

    #[test]
    fn deleting_non_empty_group_requires_explicit_move_or_ungroup() {
        let mut ledger = Ledger::default();
        let budget = BudgetId::new();
        let mut service = AccountService::new(&mut ledger);
        let group = service.create_group(budget, "Group", None).unwrap();
        let account = service
            .create(budget, "Cash", AccountType::Cash, Money::ZERO, day())
            .unwrap();
        service
            .set_account_group(account.id, Some(group.id), 0)
            .unwrap();
        assert_eq!(
            service.delete_group(group.id, None),
            Err(AccountServiceError::GroupNotEmpty)
        );
        service.delete_group(group.id, Some(None)).unwrap();
        assert_eq!(ledger.accounts.get(&account.id).unwrap().group_id, None);
    }
}
