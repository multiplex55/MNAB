//! First-run budget creation and portable database-name validation.

use crate::domain::{Budget, BudgetMonth};
use std::path::{Component, Path};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreateBudget {
    pub name: String,
    pub initial_month: BudgetMonth,
    pub currency: String,
    pub database_filename: String,
    pub starter_content: bool,
}

#[derive(Clone, Debug, thiserror::Error, Eq, PartialEq)]
pub enum CreateBudgetError {
    #[error("budget name must not be empty")]
    EmptyName,
    #[error("only USD budgets are supported")]
    UnsupportedCurrency,
    #[error("unsafe database filename")]
    UnsafeFilename,
    #[error("a budget with that filename already exists")]
    Collision,
    #[error("storage operation failed: {0}")]
    Storage(String),
}

pub trait BudgetStorage {
    type Transaction: BudgetCreationTransaction;
    fn exists(&self, filename: &str) -> Result<bool, CreateBudgetError>;
    fn begin_create(&mut self, filename: &str) -> Result<Self::Transaction, CreateBudgetError>;
}
pub trait BudgetCreationTransaction {
    fn create_database(
        &mut self,
        budget: &Budget,
        initial_month: BudgetMonth,
        currency: &str,
    ) -> Result<(), CreateBudgetError>;
    fn create_group(&mut self, name: &str, position: u32) -> Result<(), CreateBudgetError>;
    fn create_category(
        &mut self,
        group: &str,
        name: &str,
        position: u32,
    ) -> Result<(), CreateBudgetError>;
    fn commit(self) -> Result<(), CreateBudgetError>;
}

pub fn validate_filename(filename: &str) -> Result<(), CreateBudgetError> {
    if filename.is_empty() || filename != filename.trim() || filename.ends_with(['.', ' ']) {
        return Err(CreateBudgetError::UnsafeFilename);
    }
    let path = Path::new(filename);
    if path.is_absolute()
        || path.components().count() != 1
        || !matches!(path.components().next(), Some(Component::Normal(_)))
    {
        return Err(CreateBudgetError::UnsafeFilename);
    }
    if filename.chars().any(|c| {
        c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*')
    }) {
        return Err(CreateBudgetError::UnsafeFilename);
    }
    let stem = filename
        .split('.')
        .next()
        .unwrap_or("")
        .trim_end_matches(['.', ' '])
        .to_ascii_uppercase();
    let reserved = matches!(
        stem.as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    );
    if reserved {
        Err(CreateBudgetError::UnsafeFilename)
    } else {
        Ok(())
    }
}

pub fn create_budget<S: BudgetStorage>(
    storage: &mut S,
    request: CreateBudget,
) -> Result<Budget, CreateBudgetError> {
    let name = request.name.trim();
    if name.is_empty() {
        return Err(CreateBudgetError::EmptyName);
    }
    if request.currency != "USD" {
        return Err(CreateBudgetError::UnsupportedCurrency);
    }
    validate_filename(&request.database_filename)?;
    if storage.exists(&request.database_filename)? {
        return Err(CreateBudgetError::Collision);
    }
    let budget = Budget::new(name);
    let mut tx = storage.begin_create(&request.database_filename)?;
    tx.create_database(&budget, request.initial_month, "USD")?;
    if request.starter_content {
        tx.create_group("Immediate Obligations", 0)?;
        for (position, category) in ["Rent/Mortgage", "Utilities", "Groceries", "Transportation"]
            .iter()
            .enumerate()
        {
            tx.create_category(
                "Immediate Obligations",
                category,
                u32::try_from(position).expect("small starter list"),
            )?;
        }
    }
    tx.commit()?;
    Ok(budget)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_unsafe_names() {
        for n in ["../x.db", "a/b.db", "CON.db", "bad?.db", "x. ", ""] {
            assert!(validate_filename(n).is_err(), "{n}");
        }
        assert!(validate_filename("Home Budget.db").is_ok());
    }
}
