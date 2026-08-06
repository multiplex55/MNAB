//! Routed workspace surfaces. Shell chrome must not contain workspace content.
pub mod all_accounts;
pub mod categories;
pub mod inbox;
pub mod register;
pub mod reports;

use crate::app::navigation::Workspace;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkspaceRoute {
    Register,
    AllTransactions,
    Categories,
    Reports,
    Inbox,
}

#[must_use]
pub const fn route(workspace: Workspace) -> WorkspaceRoute {
    match workspace {
        Workspace::Account(_) => WorkspaceRoute::Register,
        Workspace::AllTransactions => WorkspaceRoute::AllTransactions,
        Workspace::Categories => WorkspaceRoute::Categories,
        Workspace::Reports => WorkspaceRoute::Reports,
        Workspace::Inbox => WorkspaceRoute::Inbox,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::AccountId;
    #[test]
    fn every_workspace_has_a_pure_route() {
        assert_eq!(
            route(Workspace::Account(AccountId::new())),
            WorkspaceRoute::Register
        );
        assert_eq!(
            route(Workspace::AllTransactions),
            WorkspaceRoute::AllTransactions
        );
        assert_eq!(route(Workspace::Categories), WorkspaceRoute::Categories);
        assert_eq!(route(Workspace::Reports), WorkspaceRoute::Reports);
        assert_eq!(route(Workspace::Inbox), WorkspaceRoute::Inbox);
    }
}
