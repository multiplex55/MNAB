//! Routed workspace surfaces. Shell chrome must not contain workspace content.
pub mod all_accounts;
pub mod budget;
pub mod categories;
pub mod inbox;
pub mod overview;
pub mod register;
pub mod reports;

use crate::app::navigation::Workspace;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkspaceRoute {
    Overview,
    Budget,
    Register,
    AllTransactions,
    Categories,
    Reports,
    Inbox,
}

#[must_use]
pub const fn route(workspace: Workspace) -> WorkspaceRoute {
    match workspace {
        Workspace::Overview => WorkspaceRoute::Overview,
        Workspace::Budget => WorkspaceRoute::Budget,
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
        assert_eq!(route(Workspace::Overview), WorkspaceRoute::Overview);
        assert_eq!(route(Workspace::Budget), WorkspaceRoute::Budget);
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
