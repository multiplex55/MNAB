use super::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Budget {
    pub id: BudgetId,
    pub name: String,
}
impl Budget {
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: BudgetId::new(),
            name: name.into(),
        }
    }
}

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
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Account {
    pub id: AccountId,
    pub budget_id: BudgetId,
    pub name: String,
    pub account_type: AccountType,
    pub closed: bool,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CategoryGroup {
    pub id: CategoryGroupId,
    pub budget_id: BudgetId,
    pub name: String,
    pub hidden: bool,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Category {
    pub id: CategoryId,
    pub group_id: CategoryGroupId,
    pub name: String,
    pub hidden: bool,
    pub archived: bool,
}
impl Category {
    pub fn archive(&mut self) {
        self.archived = true;
    }
    pub fn set_hidden(&mut self, hidden: bool) {
        self.hidden = hidden;
    }
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Payee {
    pub id: PayeeId,
    pub budget_id: BudgetId,
    pub name: String,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BudgetAssignment {
    pub category_id: CategoryId,
    pub month: BudgetMonth,
    /// Positive assigns money; negative unassigns it.
    pub amount: Money,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Target {
    pub id: TargetId,
    pub category_id: CategoryId,
    pub amount: Money,
    pub month: BudgetMonth,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScheduledTransaction {
    pub id: ScheduledTransactionId,
    pub account_id: AccountId,
    pub next_date: TransactionDate,
    pub amount: Money,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Reconciliation {
    pub id: ReconciliationId,
    pub account_id: AccountId,
    pub statement_date: StatementDate,
    pub balance: Money,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ImportBatch {
    pub id: ImportBatchId,
    pub account_id: AccountId,
    pub created_at: CreatedAt,
    pub candidates: Vec<ImportCandidate>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ImportCandidate {
    pub date: TransactionDate,
    pub payee: String,
    pub amount: Money,
}

#[derive(Clone, Copy, Debug)]
pub struct ClosedAccountOverride(());
impl ClosedAccountOverride {
    #[must_use]
    pub const fn confirmed() -> Self {
        Self(())
    }
}
#[derive(Clone, Copy, Debug)]
pub struct ReconciledMutationConfirmation(());
impl ReconciledMutationConfirmation {
    #[must_use]
    pub const fn confirmed() -> Self {
        Self(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MutationError {
    ClosedAccount,
    Reconciled,
    HistoricallyReferenced,
}
impl Account {
    pub fn permit_transaction(
        &self,
        confirmation: Option<ClosedAccountOverride>,
    ) -> Result<(), MutationError> {
        if self.closed && confirmation.is_none() {
            Err(MutationError::ClosedAccount)
        } else {
            Ok(())
        }
    }
}
pub fn permit_reconciled_mutation(
    reconciled: bool,
    confirmation: Option<ReconciledMutationConfirmation>,
) -> Result<(), MutationError> {
    if reconciled && confirmation.is_none() {
        Err(MutationError::Reconciled)
    } else {
        Ok(())
    }
}
/// Repository reference checks must return false before physical deletion.
pub fn permit_category_deletion(has_historical_references: bool) -> Result<(), MutationError> {
    if has_historical_references {
        Err(MutationError::HistoricallyReferenced)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn policies_and_history() {
        let bid = BudgetId::new();
        let a = Account {
            id: AccountId::new(),
            budget_id: bid,
            name: "x".into(),
            account_type: AccountType::Checking,
            closed: true,
        };
        assert_eq!(
            a.permit_transaction(None),
            Err(MutationError::ClosedAccount)
        );
        assert!(
            a.permit_transaction(Some(ClosedAccountOverride::confirmed()))
                .is_ok()
        );
        assert!(permit_reconciled_mutation(true, None).is_err());
        let id = CategoryId::new();
        let mut c = Category {
            id,
            group_id: CategoryGroupId::new(),
            name: "x".into(),
            hidden: false,
            archived: false,
        };
        c.set_hidden(true);
        c.archive();
        assert_eq!(c.id, id);
        assert!(c.hidden && c.archived);
        assert!(permit_category_deletion(true).is_err());
    }
}
