use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

use crate::error::StartupError;

#[derive(Clone, Debug)]
pub struct PortablePaths {
    pub executable: PathBuf,
    pub data: PathBuf,
    pub database: PathBuf,
    pub budgets: PathBuf,
    pub backups: PathBuf,
    pub imports: PathBuf,
    pub logs: PathBuf,
    pub settings: PathBuf,
}

impl PortablePaths {
    pub fn discover() -> Result<Self, StartupError> {
        let executable =
            std::env::current_exe().map_err(|source| StartupError::ExecutablePath { source })?;
        Self::from_executable(&executable)
    }

    pub fn from_executable(executable: &Path) -> Result<Self, StartupError> {
        let absolute = if executable.is_absolute() {
            executable.to_path_buf()
        } else {
            std::env::current_dir()
                .map_err(|source| StartupError::ExecutablePath { source })?
                .join(executable)
        };
        let parent = absolute
            .parent()
            .ok_or_else(|| StartupError::PortableStorage {
                path: absolute.clone(),
                source: std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "executable has no parent",
                ),
            })?;
        let data = parent.join("mnab-data");
        let paths = Self {
            executable: absolute,
            database: data.join("mnab.sqlite3"),
            budgets: data.join("budgets"),
            backups: data.join("backups"),
            imports: data.join("imports"),
            logs: data.join("logs"),
            settings: data.join("settings.json"),
            data,
        };
        paths.create_and_validate()?;
        Ok(paths)
    }

    fn create_and_validate(&self) -> Result<(), StartupError> {
        let attempt = || -> std::io::Result<()> {
            for directory in [
                &self.data,
                &self.budgets,
                &self.backups,
                &self.imports,
                &self.logs,
            ] {
                fs::create_dir_all(directory)?;
            }
            let probe = self
                .data
                .join(format!(".write-probe-{}", uuid::Uuid::new_v4()));
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&probe)?;
            file.write_all(b"mnab portable storage probe")?;
            file.sync_all()?;
            drop(file);
            fs::remove_file(probe)
        };
        attempt().map_err(|source| StartupError::PortableStorage {
            path: self.data.clone(),
            source,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sandbox(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!("mnab-{label}-{}", uuid::Uuid::new_v4()))
    }

    #[test]
    fn derives_all_children_with_spaces_and_unicode() {
        let root = sandbox("Portable app ü space");
        let paths = PortablePaths::from_executable(&root.join("MNAB application.exe")).unwrap();
        assert_eq!(paths.data, root.join("mnab-data"));
        assert_eq!(paths.database, paths.data.join("mnab.sqlite3"));
        assert_eq!(paths.budgets, paths.data.join("budgets"));
        assert_eq!(paths.backups, paths.data.join("backups"));
        assert_eq!(paths.imports, paths.data.join("imports"));
        assert_eq!(paths.logs, paths.data.join("logs"));
        assert_eq!(paths.settings, paths.data.join("settings.json"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn accepts_relative_executable_and_creation_is_idempotent() {
        let relative = PathBuf::from(format!(
            "target/mnab-test-{}/mnab.exe",
            uuid::Uuid::new_v4()
        ));
        let first = PortablePaths::from_executable(&relative).unwrap();
        let second = PortablePaths::from_executable(&relative).unwrap();
        assert_eq!(first.data, second.data);
        fs::remove_dir_all(first.data.parent().unwrap()).unwrap();
    }

    #[test]
    fn conflicting_data_path_reports_attempt_without_fallback() {
        let root = sandbox("conflict");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("mnab-data"), b"conflict").unwrap();
        let error = PortablePaths::from_executable(&root.join("mnab.exe")).unwrap_err();
        match error {
            StartupError::PortableStorage { path, .. } => assert_eq!(path, root.join("mnab-data")),
            other => panic!("unexpected error: {other:?}"),
        }
        assert!(!root.join("budgets").exists());
        fs::remove_dir_all(root).unwrap();
    }
}
