//! User-operation coordination. Services depend on traits, never concrete storage.

pub mod account_service;
pub mod assignment_service;
pub mod backup_service;
pub mod budget_service;
pub mod category_service;
pub mod import_service;
pub mod payee_service;
pub mod reconciliation_service;
pub mod schedule_service;
pub mod transaction_service;

use crate::{domain::Budget, error::ServiceError};

pub trait BudgetRepository {
    fn create(&mut self, budget: &Budget) -> Result<(), ServiceError>;
}

pub struct BudgetService<R> {
    repository: R,
}

impl<R: BudgetRepository> BudgetService<R> {
    pub const fn new(repository: R) -> Self {
        Self { repository }
    }

    pub fn create_budget(&mut self, name: String) -> Result<Budget, ServiceError> {
        let budget = Budget::new(name);
        self.repository.create(&budget)?;
        Ok(budget)
    }
}
