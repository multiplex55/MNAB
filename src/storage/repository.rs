//! Service-facing repository contracts and test doubles.
//!
//! These interfaces deliberately contain no `SQLite` types. A unit of work is committed once per
//! successful user command and rolled back when dropped or when `rollback` is requested.

use crate::{
    domain::{
        Account, Budget, BudgetAssignment, BudgetId, Category, CategoryGroup, ImportBatch, Payee,
        Reconciliation, ScheduledTransaction, Target, Transaction,
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
}
pub trait CategoryRepository {
    fn put_group(&mut self, value: &CategoryGroup) -> Result<(), RepositoryError>;
    fn put_category(&mut self, value: &Category) -> Result<(), RepositoryError>;
}
pub trait PayeeRepository {
    fn put_payee(&mut self, value: &Payee) -> Result<(), RepositoryError>;
}
pub trait TransactionRepository {
    fn put_transaction(&mut self, value: &Transaction) -> Result<(), RepositoryError>;
}
pub trait AssignmentRepository {
    fn put_assignment(&mut self, value: &BudgetAssignment) -> Result<(), RepositoryError>;
}
pub trait TargetRepository {
    fn put_target(&mut self, value: &Target) -> Result<(), RepositoryError>;
}
pub trait ScheduledRepository {
    fn put_scheduled(&mut self, value: &ScheduledTransaction) -> Result<(), RepositoryError>;
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
macro_rules! no_op { ($trait:ident, $($method:ident($ty:ty)),+) => { impl $trait for InMemoryRepositories { $(fn $method(&mut self, _: &$ty) -> Result<(), RepositoryError> { Ok(()) })+ } }; }
no_op!(AccountRepository, put_account(Account));
impl CategoryRepository for InMemoryRepositories {
    fn put_group(&mut self, _: &CategoryGroup) -> Result<(), RepositoryError> {
        Ok(())
    }
    fn put_category(&mut self, _: &Category) -> Result<(), RepositoryError> {
        Ok(())
    }
}
no_op!(PayeeRepository, put_payee(Payee));
no_op!(TransactionRepository, put_transaction(Transaction));
no_op!(AssignmentRepository, put_assignment(BudgetAssignment));
no_op!(TargetRepository, put_target(Target));
no_op!(ScheduledRepository, put_scheduled(ScheduledTransaction));
no_op!(ImportRepository, put_import_batch(ImportBatch));
no_op!(ReconciliationRepository, put_reconciliation(Reconciliation));
impl AuditRepository for InMemoryRepositories {
    fn append_audit(&mut self, _: &str, _: &str, _: &str) -> Result<(), RepositoryError> {
        Ok(())
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
