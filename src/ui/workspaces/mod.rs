//! Routed workspace surfaces. Shell chrome must not contain workspace content.
pub mod all_accounts;
pub mod budget;
pub mod inbox;
pub mod register;
pub mod reports;

use crate::app::navigation::Workspace;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkspaceRoute {
    Budget,
    Register,
    AllAccounts,
    Reports,
    Inbox,
}

#[must_use]
pub const fn route(workspace: Workspace) -> WorkspaceRoute {
    match workspace {
        Workspace::Budget => WorkspaceRoute::Budget,
        Workspace::Account(_) => WorkspaceRoute::Register,
        Workspace::AllAccounts => WorkspaceRoute::AllAccounts,
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
        assert_eq!(route(Workspace::Budget), WorkspaceRoute::Budget);
        assert_eq!(
            route(Workspace::Account(AccountId::new())),
            WorkspaceRoute::Register
        );
        assert_eq!(route(Workspace::AllAccounts), WorkspaceRoute::AllAccounts);
        assert_eq!(route(Workspace::Reports), WorkspaceRoute::Reports);
        assert_eq!(route(Workspace::Inbox), WorkspaceRoute::Inbox);
    }
}
