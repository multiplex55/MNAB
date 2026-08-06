use std::path::Path;

use crate::app::settings::SettingsSession;
use crate::{app::navigation::Workspace, domain::AccountId};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StartupAccount {
    pub id: AccountId,
    pub favorite: bool,
    pub closed: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StartupDestination {
    Workspace(Workspace),
    AccountOnboarding,
}

/// Resolve after the fixed database is open. `accounts` must be in account-tree order.
pub fn resolve_destination(last: Option<&str>, accounts: &[StartupAccount]) -> StartupDestination {
    let restored = last
        .and_then(|raw| raw.parse::<AccountId>().ok())
        .and_then(|id| accounts.iter().find(|a| a.id == id && !a.closed));
    let account = restored
        .or_else(|| accounts.iter().find(|a| a.favorite && !a.closed))
        .or_else(|| accounts.iter().find(|a| !a.closed));
    account.map_or(StartupDestination::AccountOnboarding, |a| {
        StartupDestination::Workspace(Workspace::Account(a.id))
    })
}

/// Facts captured before the previous clean marker is removed.  Startup policy is
/// explicit data, rather than a logging side effect.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StartupContext {
    pub marker_was_absent: bool,
    pub fixed_database_exists: bool,
}

impl StartupContext {
    #[must_use]
    pub fn capture(marker: &Path, _settings: &SettingsSession) -> Self {
        let fixed_database_exists = marker
            .parent()
            .is_some_and(|data| data.join("mnab.sqlite3").is_file());
        Self {
            marker_was_absent: !marker.exists(),
            fixed_database_exists,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marker_presence_and_fixed_database_are_captured() {
        let dir = tempfile::tempdir().unwrap();
        let settings_path = dir.path().join("settings.json");
        let settings = SettingsSession::load(&settings_path);
        let marker = dir.path().join(".clean-shutdown");
        assert!(StartupContext::capture(&marker, &settings).marker_was_absent);
        std::fs::write(dir.path().join("mnab.sqlite3"), "not sqlite").unwrap();
        std::fs::write(&marker, "clean").unwrap();
        let context = StartupContext::capture(&marker, &settings);
        assert!(!context.marker_was_absent);
        assert!(context.fixed_database_exists);
    }

    #[test]
    fn destination_restores_then_falls_back_in_tree_order() {
        let first = StartupAccount {
            id: AccountId::new(),
            favorite: false,
            closed: false,
        };
        let favorite = StartupAccount {
            id: AccountId::new(),
            favorite: true,
            closed: false,
        };
        assert_eq!(
            resolve_destination(Some(&first.id.to_string()), &[first, favorite]),
            StartupDestination::Workspace(Workspace::Account(first.id))
        );
        assert_eq!(
            resolve_destination(Some(&AccountId::new().to_string()), &[first, favorite]),
            StartupDestination::Workspace(Workspace::Account(favorite.id))
        );
        assert_eq!(
            resolve_destination(None, &[]),
            StartupDestination::AccountOnboarding
        );
    }
}
