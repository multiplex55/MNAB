use serde::{Deserialize, Serialize};
use thiserror::Error;
use time::{Date, OffsetDateTime};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TransactionDate(pub Date);
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct StatementDate(pub Date);
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CreatedAt(pub OffsetDateTime);
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ModifiedAt(pub OffsetDateTime);

#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub enum MonthError {
    #[error("month must be between 1 and 12")]
    InvalidMonth,
    #[error("month is outside the supported year range")]
    OutOfRange,
}
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct BudgetMonth {
    year: i32,
    month: u8,
}
impl BudgetMonth {
    pub fn new(year: i32, month: u8) -> Result<Self, MonthError> {
        if !(1..=12).contains(&month) {
            Err(MonthError::InvalidMonth)
        } else {
            Ok(Self { year, month })
        }
    }
    #[must_use]
    pub const fn year(self) -> i32 {
        self.year
    }
    #[must_use]
    pub const fn month(self) -> u8 {
        self.month
    }
    pub fn next(self) -> Result<Self, MonthError> {
        if self.month == 12 {
            Self::new(self.year.checked_add(1).ok_or(MonthError::OutOfRange)?, 1)
        } else {
            Self::new(self.year, self.month + 1)
        }
    }
    pub fn previous(self) -> Result<Self, MonthError> {
        if self.month == 1 {
            Self::new(self.year.checked_sub(1).ok_or(MonthError::OutOfRange)?, 12)
        } else {
            Self::new(self.year, self.month - 1)
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn transitions() {
        let dec = BudgetMonth::new(2025, 12).unwrap();
        assert_eq!(dec.next().unwrap(), BudgetMonth::new(2026, 1).unwrap());
        assert_eq!(dec.next().unwrap().previous().unwrap(), dec);
        assert!(BudgetMonth::new(1, 0).is_err());
        assert!(BudgetMonth::new(1, 13).is_err());
    }
}
