//! Service-facing repository contracts and test doubles.
//!
//! These interfaces deliberately contain no `SQLite` types. A unit of work is committed once per
//! successful user command and rolled back when dropped or when `rollback` is requested.

use crate::{
    domain::{
        Account, AccountId, Budget, BudgetAssignment, BudgetId, Category, CategoryGroup,
        CategoryId, ImportBatch, MerchantRule, Payee, PayeeId, Reconciliation,
        ScheduledTransaction, ScheduledTransactionId, Target, TargetId, Transaction, TransactionId,
    },
    error::RepositoryError,
};
use std::collections::HashMap;

pub trait BudgetRepository {
    fn create_budget(&mut self, value: &Budget) -> Result<(), RepositoryError>;
    fn budget(&mut self, id: BudgetId) -> Result<Option<Budget>, RepositoryError>;
}
pub trait AccountRepository {
    fn put_account(&mut self, value: &Account) -> Result<(), RepositoryError>;
    fn account(&mut self, id: AccountId) -> Result<Option<Account>, RepositoryError>;
}
pub trait CategoryRepository {
    fn put_group(&mut self, value: &CategoryGroup) -> Result<(), RepositoryError>;
    fn put_category(&mut self, value: &Category) -> Result<(), RepositoryError>;
    fn category(&mut self, id: CategoryId) -> Result<Option<Category>, RepositoryError>;
    fn category_is_used(&mut self, id: CategoryId) -> Result<bool, RepositoryError>;
    fn delete_category(&mut self, id: CategoryId) -> Result<(), RepositoryError>;
}
pub trait PayeeRepository {
    fn put_payee(&mut self, value: &Payee) -> Result<(), RepositoryError>;
    fn payee(&mut self, id: PayeeId) -> Result<Option<Payee>, RepositoryError>;
    fn payee_is_used(&mut self, id: PayeeId) -> Result<bool, RepositoryError>;
    fn delete_payee(&mut self, id: PayeeId) -> Result<(), RepositoryError>;
}
pub trait TransactionRepository {
    fn put_transaction(&mut self, value: &Transaction) -> Result<(), RepositoryError>;
    fn transaction(&mut self, id: TransactionId) -> Result<Option<Transaction>, RepositoryError>;
    fn delete_transaction(&mut self, id: TransactionId) -> Result<(), RepositoryError>;
}
pub trait AssignmentRepository {
    fn put_assignment(&mut self, value: &BudgetAssignment) -> Result<(), RepositoryError>;
    fn assignment(
        &mut self,
        category: CategoryId,
        month: crate::domain::BudgetMonth,
    ) -> Result<Option<BudgetAssignment>, RepositoryError>;
}
pub trait TargetRepository {
    fn put_target(&mut self, value: &Target) -> Result<(), RepositoryError>;
    fn delete_target(&mut self, id: TargetId) -> Result<(), RepositoryError>;
}
pub trait ScheduledRepository {
    fn put_scheduled(&mut self, value: &ScheduledTransaction) -> Result<(), RepositoryError>;
    fn delete_scheduled(&mut self, id: ScheduledTransactionId) -> Result<(), RepositoryError>;
}
pub trait ImportRepository {
    fn put_import_batch(&mut self, value: &ImportBatch) -> Result<(), RepositoryError>;
}
pub trait ReconciliationRepository {
    fn put_reconciliation(&mut self, value: &Reconciliation) -> Result<(), RepositoryError>;
}
pub trait AuditRepository {
    fn append_audit(
        &mut self,
        entity: &str,
        record_id: &str,
        operation: &str,
    ) -> Result<(), RepositoryError>;
}
pub trait InboxRepository {
    /// Toggles the durable dismissal marker and returns whether the row was previously dismissed.
    fn toggle_failure_dismissal(&mut self, id: &str) -> Result<Option<bool>, RepositoryError>;
}
pub trait MerchantRuleRepository {
    fn put_merchant_rule(&mut self, value: &MerchantRule) -> Result<(), RepositoryError>;
    fn merchant_rules(&mut self) -> Result<Vec<MerchantRule>, RepositoryError>;
}

pub trait Repositories:
    BudgetRepository
    + AccountRepository
    + CategoryRepository
    + PayeeRepository
    + TransactionRepository
    + AssignmentRepository
    + TargetRepository
    + ScheduledRepository
    + ImportRepository
    + ReconciliationRepository
    + AuditRepository
    + InboxRepository
{
}
impl<T> Repositories for T where
    T: BudgetRepository
        + AccountRepository
        + CategoryRepository
        + PayeeRepository
        + TransactionRepository
        + AssignmentRepository
        + TargetRepository
        + ScheduledRepository
        + ImportRepository
        + ReconciliationRepository
        + AuditRepository
        + InboxRepository
{
}

pub trait UnitOfWork {
    type Repositories: Repositories;
    fn repositories(&mut self) -> &mut Self::Repositories;
    fn commit(self) -> Result<(), RepositoryError>;
    fn rollback(self) -> Result<(), RepositoryError>;
}
pub trait UnitOfWorkFactory {
    type Work<'a>: UnitOfWork
    where
        Self: 'a;
    fn begin(&mut self) -> Result<Self::Work<'_>, RepositoryError>;
}

/// Lightweight deterministic repository for service unit tests.
#[derive(Default)]
pub struct InMemoryRepositories {
    pub budgets: HashMap<BudgetId, Budget>,
    pub accounts: HashMap<AccountId, Account>,
    pub groups: HashMap<crate::domain::CategoryGroupId, CategoryGroup>,
    pub categories: HashMap<CategoryId, Category>,
    pub payees: HashMap<PayeeId, Payee>,
    pub transactions: HashMap<TransactionId, Transaction>,
    pub assignments: HashMap<(CategoryId, crate::domain::BudgetMonth), BudgetAssignment>,
    pub targets: HashMap<TargetId, Target>,
    pub schedules: HashMap<ScheduledTransactionId, ScheduledTransaction>,
    pub imports: HashMap<crate::domain::ImportBatchId, ImportBatch>,
    pub reconciliations: HashMap<crate::domain::ReconciliationId, Reconciliation>,
    pub audit: Vec<(String, String, String)>,
    pub failure_dismissals: HashMap<String, bool>,
    pub merchant_rules: Vec<MerchantRule>,
}
impl MerchantRuleRepository for InMemoryRepositories {
    fn put_merchant_rule(&mut self, value: &MerchantRule) -> Result<(), RepositoryError> {
        if let Some(existing) = self.merchant_rules.iter_mut().find(|r| {
            r.normalized_merchant == value.normalized_merchant
                && r.account_id == value.account_id
                && r.origin == value.origin
        }) {
            *existing = value.clone();
        } else {
            self.merchant_rules.push(value.clone());
        }
        Ok(())
    }
    fn merchant_rules(&mut self) -> Result<Vec<MerchantRule>, RepositoryError> {
        Ok(self.merchant_rules.clone())
    }
}
impl BudgetRepository for InMemoryRepositories {
    fn create_budget(&mut self, v: &Budget) -> Result<(), RepositoryError> {
        self.budgets.insert(v.id, v.clone());
        Ok(())
    }
    fn budget(&mut self, id: BudgetId) -> Result<Option<Budget>, RepositoryError> {
        Ok(self.budgets.get(&id).cloned())
    }
}
impl AccountRepository for InMemoryRepositories {
    fn put_account(&mut self, v: &Account) -> Result<(), RepositoryError> {
        self.accounts.insert(v.id, v.clone());
        Ok(())
    }
    fn account(&mut self, id: AccountId) -> Result<Option<Account>, RepositoryError> {
        Ok(self.accounts.get(&id).cloned())
    }
}
impl CategoryRepository for InMemoryRepositories {
    fn put_group(&mut self, v: &CategoryGroup) -> Result<(), RepositoryError> {
        self.groups.insert(v.id, v.clone());
        Ok(())
    }
    fn put_category(&mut self, v: &Category) -> Result<(), RepositoryError> {
        self.categories.insert(v.id, v.clone());
        Ok(())
    }
    fn category(&mut self, id: CategoryId) -> Result<Option<Category>, RepositoryError> {
        Ok(self.categories.get(&id).cloned())
    }
    fn category_is_used(&mut self, id: CategoryId) -> Result<bool, RepositoryError> {
        Ok(self.transactions.values().any(|t| match &t.body {
            crate::domain::TransactionBody::Categorized { category_id } => *category_id == id,
            crate::domain::TransactionBody::Split { lines } => {
                lines.iter().any(|l| l.category_id == id)
            }
            crate::domain::TransactionBody::OpeningBalance { category_id } => {
                *category_id == Some(id)
            }
            _ => false,
        }))
    }
    fn delete_category(&mut self, id: CategoryId) -> Result<(), RepositoryError> {
        self.categories.remove(&id);
        Ok(())
    }
}
impl PayeeRepository for InMemoryRepositories {
    fn put_payee(&mut self, v: &Payee) -> Result<(), RepositoryError> {
        self.payees.insert(v.id, v.clone());
        Ok(())
    }
    fn payee(&mut self, id: PayeeId) -> Result<Option<Payee>, RepositoryError> {
        Ok(self.payees.get(&id).cloned())
    }
    fn payee_is_used(&mut self, id: PayeeId) -> Result<bool, RepositoryError> {
        Ok(self.transactions.values().any(|t| t.payee_id == Some(id)))
    }
    fn delete_payee(&mut self, id: PayeeId) -> Result<(), RepositoryError> {
        self.payees.remove(&id);
        Ok(())
    }
}
impl TransactionRepository for InMemoryRepositories {
    fn put_transaction(&mut self, v: &Transaction) -> Result<(), RepositoryError> {
        self.transactions.insert(v.id, v.clone());
        Ok(())
    }
    fn transaction(&mut self, id: TransactionId) -> Result<Option<Transaction>, RepositoryError> {
        Ok(self.transactions.get(&id).cloned())
    }
    fn delete_transaction(&mut self, id: TransactionId) -> Result<(), RepositoryError> {
        self.transactions.remove(&id);
        Ok(())
    }
}
impl AssignmentRepository for InMemoryRepositories {
    fn put_assignment(&mut self, v: &BudgetAssignment) -> Result<(), RepositoryError> {
        self.assignments.insert((v.category_id, v.month), v.clone());
        Ok(())
    }
    fn assignment(
        &mut self,
        c: CategoryId,
        m: crate::domain::BudgetMonth,
    ) -> Result<Option<BudgetAssignment>, RepositoryError> {
        Ok(self.assignments.get(&(c, m)).cloned())
    }
}
impl TargetRepository for InMemoryRepositories {
    fn put_target(&mut self, v: &Target) -> Result<(), RepositoryError> {
        self.targets.insert(v.id, v.clone());
        Ok(())
    }
    fn delete_target(&mut self, id: TargetId) -> Result<(), RepositoryError> {
        self.targets.remove(&id);
        Ok(())
    }
}
impl ScheduledRepository for InMemoryRepositories {
    fn put_scheduled(&mut self, v: &ScheduledTransaction) -> Result<(), RepositoryError> {
        self.schedules.insert(v.id, v.clone());
        Ok(())
    }
    fn delete_scheduled(&mut self, id: ScheduledTransactionId) -> Result<(), RepositoryError> {
        self.schedules.remove(&id);
        Ok(())
    }
}
impl ImportRepository for InMemoryRepositories {
    fn put_import_batch(&mut self, v: &ImportBatch) -> Result<(), RepositoryError> {
        self.imports.insert(v.id, v.clone());
        Ok(())
    }
}
impl ReconciliationRepository for InMemoryRepositories {
    fn put_reconciliation(&mut self, v: &Reconciliation) -> Result<(), RepositoryError> {
        self.reconciliations.insert(v.id, v.clone());
        Ok(())
    }
}
impl AuditRepository for InMemoryRepositories {
    fn append_audit(&mut self, e: &str, id: &str, op: &str) -> Result<(), RepositoryError> {
        self.audit.push((e.into(), id.into(), op.into()));
        Ok(())
    }
}
impl InboxRepository for InMemoryRepositories {
    fn toggle_failure_dismissal(&mut self, id: &str) -> Result<Option<bool>, RepositoryError> {
        let before = self.failure_dismissals.get(id).copied().unwrap_or(false);
        self.failure_dismissals.insert(id.to_owned(), !before);
        Ok(Some(before))
    }
}

/// Safe row-conversion failure context; values from financial columns are intentionally omitted.
#[derive(Debug, Clone, Eq, PartialEq, thiserror::Error)]
#[error("invalid row in {table} for record {record_id}: {reason}")]
pub struct RowConversionError {
    pub table: &'static str,
    pub record_id: String,
    pub reason: &'static str,
}

impl RowConversionError {
    #[must_use]
    pub fn new(table: &'static str, record_id: impl Into<String>, reason: &'static str) -> Self {
        Self {
            table,
            record_id: record_id.into(),
            reason,
        }
    }
}
