use std::path::{Path, PathBuf};

use crate::app::settings::SettingsSession;

/// Facts captured before the previous clean marker is removed.  Startup policy is
/// explicit data, rather than a logging side effect.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StartupContext {
    pub marker_was_absent: bool,
    pub last_successfully_opened_budget: Option<PathBuf>,
}

impl StartupContext {
    #[must_use]
    pub fn capture(marker: &Path, settings: &SettingsSession) -> Self {
        Self {
            marker_was_absent: !marker.exists(),
            last_successfully_opened_budget: settings.value().last_opened_budget.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marker_presence_and_last_budget_are_captured() {
        let dir = tempfile::tempdir().unwrap();
        let settings_path = dir.path().join("settings.json");
        let mut settings = SettingsSession::load(&settings_path);
        settings.value_mut().last_opened_budget = Some(dir.path().join("last.sqlite3"));
        let marker = dir.path().join(".clean-shutdown");
        assert!(StartupContext::capture(&marker, &settings).marker_was_absent);
        std::fs::write(&marker, "clean").unwrap();
        let context = StartupContext::capture(&marker, &settings);
        assert!(!context.marker_was_absent);
        assert_eq!(
            context.last_successfully_opened_budget,
            settings.value().last_opened_budget
        );
    }
}
