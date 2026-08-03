//! User-operation coordination. Services depend on traits, never concrete storage.

use crate::{domain::Budget, error::ServiceError};

pub trait BudgetRepository {
    fn create(&mut self, budget: &Budget) -> Result<(), ServiceError>;
}

#[derive(Debug)]
pub enum AppCommand {
    CreateBudget { name: String },
    Exit,
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
