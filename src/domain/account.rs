use super::{AccountGroupId, AccountId, BudgetId, Money};
use serde::{Deserialize, Serialize};

/// The account's behavior class. Balances deliberately do not live on `Account`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum AccountType {
    Checking,
    Savings,
    Cash,
    CreditCard,
    Loan,
    Asset,
    Liability,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum AccountClassification {
    OnBudget,
    Tracking,
}

impl AccountType {
    #[must_use]
    pub const fn classification(self) -> AccountClassification {
        match self {
            Self::Checking | Self::Savings | Self::Cash | Self::CreditCard => {
                AccountClassification::OnBudget
            }
            Self::Asset | Self::Liability | Self::Loan => AccountClassification::Tracking,
        }
    }

    /// Converts the positive magnitude entered by the opening wizard to ledger sign.
    pub fn opening_amount(self, magnitude: Money) -> Result<Money, super::MoneyError> {
        match self {
            Self::CreditCard | Self::Loan | Self::Liability => magnitude.checked_neg(),
            _ => Ok(magnitude),
        }
    }
}

/// Account groups form a budget-local tree for account navigation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AccountGroup {
    pub id: AccountGroupId,
    pub budget_id: BudgetId,
    pub parent_group_id: Option<AccountGroupId>,
    pub name: String,
    pub sort_order: i64,
    pub collapsed: bool,
}

impl AccountGroup {
    #[must_use]
    pub fn new(budget_id: BudgetId, name: impl Into<String>) -> Self {
        Self {
            id: AccountGroupId::new(),
            budget_id,
            parent_group_id: None,
            name: name.into(),
            sort_order: 0,
            collapsed: false,
        }
    }
}

/// Persisted account metadata/state. Every balance is reconstructed from transactions.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Account {
    pub id: AccountId,
    pub budget_id: BudgetId,
    pub group_id: Option<AccountGroupId>,
    pub name: String,
    pub account_type: AccountType,
    pub closed: bool,
    pub note: Option<String>,
    pub sort_order: i64,
    pub favorite: bool,
}

impl Account {
    #[must_use]
    pub fn new(budget_id: BudgetId, name: impl Into<String>, account_type: AccountType) -> Self {
        Self {
            id: AccountId::new(),
            budget_id,
            group_id: None,
            name: name.into(),
            account_type,
            closed: false,
            note: None,
            sort_order: 0,
            favorite: false,
        }
    }
    #[must_use]
    pub const fn classification(&self) -> AccountClassification {
        self.account_type.classification()
    }
}
