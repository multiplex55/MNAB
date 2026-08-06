use crate::domain::AccountId;

/// Stable, account-centric destinations. Calendar months belong to report/date filters,
/// never to application navigation.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Workspace {
    Account(AccountId),
    #[default]
    AllTransactions,
    Categories,
    Reports,
    Inbox,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Navigation {
    pub workspace: Workspace,
}

impl Navigation {
    pub const fn new(workspace: Workspace) -> Self {
        Self { workspace }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn navigation_defaults_to_all_transactions_without_a_month() {
        assert_eq!(Navigation::default().workspace, Workspace::AllTransactions);
    }

    #[test]
    fn navigation_and_inspector_do_not_reference_monthly_assignment_workspace() {
        let sources = include_str!("../ui/inspector.rs");
        assert!(!sources.contains("BudgetMonth"));
        assert!(!sources.contains("Workspace::Budget"));
    }
}
