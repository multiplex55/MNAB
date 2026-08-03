//! `SQLite` repository implementations. Connection details remain private to this module.

use std::path::Path;

use rusqlite::Connection;

use crate::{domain::Budget, error::ServiceError, service::BudgetRepository};

pub struct SqliteBudgetRepository {
    connection: Connection,
}

impl SqliteBudgetRepository {
    pub fn open(path: &Path) -> Result<Self, rusqlite::Error> {
        Connection::open(path).map(|connection| Self { connection })
    }
}

impl BudgetRepository for SqliteBudgetRepository {
    fn create(&mut self, budget: &Budget) -> Result<(), ServiceError> {
        self.connection
            .execute(
                "INSERT INTO budgets (id, name) VALUES (?1, ?2)",
                (budget.id.to_string(), &budget.name),
            )
            .map(|_| ())
            .map_err(|source| ServiceError::Failed {
                source: Box::new(source),
            })
    }
}
