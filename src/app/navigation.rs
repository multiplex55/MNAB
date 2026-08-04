use crate::domain::{AccountId, BudgetMonth};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Workspace {
    #[default]
    Budget,
    Reports,
    AllAccounts,
    Inbox,
    Account(AccountId),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Navigation {
    pub workspace: Workspace,
    pub month: BudgetMonth,
}
impl Default for Navigation {
    fn default() -> Self {
        Self {
            workspace: Workspace::Budget,
            month: BudgetMonth::new(2026, 1).expect("valid month"),
        }
    }
}
