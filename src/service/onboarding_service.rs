//! Atomic first-run aggregate creation. Persistence adapters can wrap the same operation in their
//! database transaction; this in-memory form makes the all-or-nothing contract explicit/testable.
use crate::{
    domain::*,
    service::account_service::{AccountService, AccountServiceError, Ledger},
};

#[derive(Clone, Debug)]
pub struct OnboardingRequest {
    pub budget_name: String,
    pub account_name: String,
    pub account_type: AccountType,
    pub opening_magnitude: Money,
    pub balance_date: TransactionDate,
    pub group_name: String,
    pub note: Option<String>,
    pub categories: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct OnboardingStore {
    pub budgets: Vec<Budget>,
    pub category_groups: Vec<CategoryGroup>,
    pub categories: Vec<Category>,
    pub ledger: Ledger,
}

#[derive(Debug, thiserror::Error, Eq, PartialEq)]
pub enum OnboardingError {
    #[error("budget and account names are required")]
    RequiredName,
    #[error("account setup failed: {0}")]
    Account(#[from] AccountServiceError),
}

/// Creates budget metadata, default account groups, the first account and opening balance, and
/// selected starter categories against a staged copy, then swaps it in with one commit.
pub fn commit(
    store: &mut OnboardingStore,
    request: OnboardingRequest,
) -> Result<(BudgetId, AccountId), OnboardingError> {
    if request.budget_name.trim().is_empty() || request.account_name.trim().is_empty() {
        return Err(OnboardingError::RequiredName);
    }
    let mut staged = store.clone();
    let budget = Budget::new(request.budget_name.trim());
    let account_id;
    {
        let mut accounts = AccountService::new(&mut staged.ledger);
        let group = accounts.create_group(budget.id, request.group_name, None)?;
        let account = accounts.create(
            budget.id,
            request.account_name,
            request.account_type,
            request.opening_magnitude,
            request.balance_date,
        )?;
        accounts.set_account_group(account.id, Some(group.id), 0)?;
        accounts.annotate(account.id, request.note.filter(|n| !n.trim().is_empty()))?;
        account_id = account.id;
    }
    // These useful defaults exist even when the user chooses no starter categories.
    for (position, name) in ["Cash Accounts", "Credit Accounts", "Tracking Accounts"]
        .iter()
        .enumerate()
    {
        if !staged
            .ledger
            .account_groups
            .values()
            .any(|g| g.budget_id == budget.id && g.name == *name)
        {
            let mut g = AccountGroup::new(budget.id, *name);
            g.sort_order = position as i64;
            staged.ledger.account_groups.insert(g.id, g);
        }
    }
    if !request.categories.is_empty() {
        let group = CategoryGroup {
            id: CategoryGroupId::new(),
            budget_id: budget.id,
            name: "Starter Categories".into(),
            hidden: false,
        };
        staged
            .categories
            .extend(request.categories.into_iter().map(|name| Category {
                id: CategoryId::new(),
                group_id: group.id,
                name,
                hidden: false,
                archived: false,
            }));
        staged.category_groups.push(group);
    }
    let budget_id = budget.id;
    staged.budgets.push(budget);
    *store = staged;
    Ok((budget_id, account_id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::date;
    fn request() -> OnboardingRequest {
        OnboardingRequest {
            budget_name: "Home".into(),
            account_name: "Card".into(),
            account_type: AccountType::CreditCard,
            opening_magnitude: Money::from_minor_units(12500),
            balance_date: TransactionDate(date!(2026 - 08 - 06)),
            group_name: "Credit Accounts".into(),
            note: None,
            categories: vec!["Groceries".into()],
        }
    }
    #[test]
    fn commit_is_complete_and_navigable() {
        let mut s = OnboardingStore::default();
        let (budget, account) = commit(&mut s, request()).unwrap();
        assert_eq!(s.budgets[0].id, budget);
        assert!(s.ledger.accounts.contains_key(&account));
        assert_eq!(
            s.ledger.transactions.values().next().unwrap().amount,
            Money::from_minor_units(-12500)
        );
        assert_eq!(s.categories.len(), 1);
        assert!(s.ledger.account_groups.len() >= 3);
    }
    #[test]
    fn validation_failure_leaves_store_unchanged() {
        let mut s = OnboardingStore::default();
        let mut r = request();
        r.account_name.clear();
        assert_eq!(commit(&mut s, r), Err(OnboardingError::RequiredName));
        assert!(s.budgets.is_empty());
        assert!(s.ledger.accounts.is_empty());
    }
}
