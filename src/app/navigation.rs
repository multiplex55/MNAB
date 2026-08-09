use crate::{
    app::{
        search::{RegisterSort, SearchAst},
        view_model::RegisterScope,
    },
    domain::AccountId,
};

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

/// Typed navigation payload shared by inbox, palette, search, and saved views.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RegisterDestination {
    pub scope: RegisterScope,
    pub filter: SearchAst,
    pub sort: RegisterSort,
}

impl RegisterDestination {
    #[must_use]
    pub fn request(
        &self,
        budget_id: crate::domain::BudgetId,
        page_size: usize,
    ) -> crate::app::view_model::RegisterRequest {
        crate::app::view_model::RegisterRequest {
            budget_id,
            scope: self.scope,
            filter: crate::app::search::register_filter(&self.filter),
            sort_field: self.sort.field,
            sort_direction: self.sort.direction,
            page_size,
            cursor: None,
        }
    }
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
