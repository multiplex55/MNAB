use std::{error::Error as StdError, path::PathBuf};

use thiserror::Error;

pub trait UserFacingError {
    fn user_summary(&self) -> String;
}

#[derive(Debug, Error)]
pub enum StartupError {
    #[error("portable storage at {path} is unavailable: {source}")]
    PortableStorage {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("could not locate the running executable: {source}")]
    ExecutablePath {
        #[source]
        source: std::io::Error,
    },
    #[error("logging could not be initialized at {path}: {source}")]
    Logging {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

impl StartupError {
    #[must_use]
    pub fn affected_path(&self) -> Option<&std::path::Path> {
        match self {
            Self::PortableStorage { path, .. } | Self::Logging { path, .. } => Some(path),
            Self::ExecutablePath { .. } => None,
        }
    }
}

impl UserFacingError for StartupError {
    fn user_summary(&self) -> String {
        match self {
            Self::PortableStorage { path, .. } => format!(
                "MNAB must store all data beside the application. The location '{}' is not writable. Move MNAB to a user-writable folder and try again.",
                path.display()
            ),
            Self::ExecutablePath { .. } => {
                "MNAB could not determine its installation location. Please reinstall it.".into()
            }
            Self::Logging { path, .. } => format!(
                "MNAB could not create its log at '{}'. Check folder permissions.",
                path.display()
            ),
        }
    }
}

macro_rules! sourced_error {
    ($name:ident, $message:literal) => {
        #[derive(Debug, Error)]
        pub enum $name {
            #[error($message)]
            Failed {
                #[source]
                source: Box<dyn StdError + Send + Sync>,
            },
        }
    };
}

sourced_error!(ValidationError, "validation failed");
sourced_error!(RepositoryError, "repository operation failed");
sourced_error!(MigrationError, "database migration failed");
sourced_error!(ImportError, "statement import failed");
sourced_error!(BackupError, "budget backup failed");
sourced_error!(ServiceError, "operation failed");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_hides_source_but_diagnostic_retains_it() {
        let error = StartupError::PortableStorage {
            path: PathBuf::from("C:/Portable/MNAB/mnab-data"),
            source: std::io::Error::new(std::io::ErrorKind::PermissionDenied, "secret OS detail"),
        };
        assert!(!error.user_summary().contains("secret OS detail"));
        assert!(format!("{error:?}").contains("secret OS detail"));
        assert!(error.source().is_some());
    }
}
