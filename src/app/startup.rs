use std::path::Path;

use crate::app::settings::SettingsSession;

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
}
