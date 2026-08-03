//! Read-only projection queries. Command repositories intentionally do not implement this type.
use crate::{
    domain::{AccountId, BudgetId, Money},
    error::RepositoryError,
};
use rusqlite::Connection;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RegisterRow {
    pub transaction_id: String,
    pub date: String,
    pub amount: Money,
    pub cleared_state: String,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BudgetRow {
    pub category_id: String,
    pub assigned: Money,
}

pub struct QueryStore<'a> {
    connection: &'a Connection,
}
impl<'a> QueryStore<'a> {
    #[must_use]
    pub const fn new(connection: &'a Connection) -> Self {
        Self { connection }
    }
    pub fn register(&self, account: AccountId) -> Result<Vec<RegisterRow>, RepositoryError> {
        let mut statement=self.connection.prepare("SELECT id,transaction_date,amount,cleared_state FROM transactions WHERE account_id=?1 AND archived=0 ORDER BY transaction_date DESC,id DESC").map_err(repo)?;
        statement
            .query_map([account.to_string()], |r| {
                Ok(RegisterRow {
                    transaction_id: r.get(0)?,
                    date: r.get(1)?,
                    amount: Money::from_minor_units(r.get(2)?),
                    cleared_state: r.get(3)?,
                })
            })
            .map_err(repo)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(repo)
    }
    pub fn budget_month(
        &self,
        budget: BudgetId,
        month: &str,
    ) -> Result<Vec<BudgetRow>, RepositoryError> {
        let mut statement=self.connection.prepare("SELECT category_id,amount FROM budget_assignments WHERE budget_id=?1 AND budget_month=?2 ORDER BY category_id").map_err(repo)?;
        statement
            .query_map((budget.to_string(), month), |r| {
                Ok(BudgetRow {
                    category_id: r.get(0)?,
                    assigned: Money::from_minor_units(r.get(1)?),
                })
            })
            .map_err(repo)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(repo)
    }
}
fn repo<E: std::error::Error + Send + Sync + 'static>(source: E) -> RepositoryError {
    RepositoryError::Failed {
        source: Box::new(source),
    }
}
