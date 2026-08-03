//! Portable, deliberately small application settings.
//!
//! This model is an allowlist. Transient navigation and layout state (including
//! workspace, selected account/month, searches, filters, and pane widths) must
//! never be added merely because it is present in `AppState`.

use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
};

pub const SETTINGS_VERSION: u32 = 1;
pub const DEFAULT_PALETTE_SHORTCUT: &str = "Ctrl+P";
const TEMP_PREFIX: &str = ".settings.json.tmp-";

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WindowBounds {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Default for WindowBounds {
    fn default() -> Self {
        Self {
            x: 80.0,
            y: 80.0,
            width: 1280.0,
            height: 800.0,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DisplayDensity {
    #[default]
    Comfortable,
    Compact,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct RegisterColumns {
    pub order: Vec<String>,
    pub widths: Vec<f32>,
}
impl Default for RegisterColumns {
    fn default() -> Self {
        Self {
            order: vec![
                "date", "payee", "category", "memo", "outflow", "inflow", "balance",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
            widths: vec![90.0, 180.0, 180.0, 200.0, 100.0, 100.0, 110.0],
        }
    }
}

/// Only inbox tuning is persisted here; this is not a general saved-filter model.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct InboxThresholds {
    pub duplicate_window_days: u16,
    pub upcoming_days: u16,
}
impl Default for InboxThresholds {
    fn default() -> Self {
        Self {
            duplicate_window_days: 7,
            upcoming_days: 14,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub version: u32,
    pub last_opened_budget: Option<PathBuf>,
    pub window: WindowBounds,
    pub inspector_visible: bool,
    pub register_columns: RegisterColumns,
    pub theme: Theme,
    pub display_density: DisplayDensity,
    pub command_palette_shortcut: String,
    pub inbox_thresholds: InboxThresholds,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            version: SETTINGS_VERSION,
            last_opened_budget: None,
            window: WindowBounds::default(),
            inspector_visible: true,
            register_columns: RegisterColumns::default(),
            theme: Theme::default(),
            display_density: DisplayDensity::default(),
            command_palette_shortcut: DEFAULT_PALETTE_SHORTCUT.into(),
            inbox_thresholds: InboxThresholds::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LoadStatus {
    Missing,
    Supported,
    Malformed,
    UnsupportedFuture,
}

pub struct SettingsSession {
    path: PathBuf,
    value: Settings,
    status: LoadStatus,
    read_only: bool,
}

impl SettingsSession {
    pub fn load(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        cleanup_abandoned_temps(&path);
        let Ok(bytes) = fs::read(&path) else {
            return Self {
                path,
                value: Settings::default(),
                status: LoadStatus::Missing,
                read_only: false,
            };
        };
        let parsed_value: serde_json::Value = match serde_json::from_slice(&bytes) {
            Ok(value) => value,
            Err(_) => {
                return Self {
                    path,
                    value: Settings::default(),
                    status: LoadStatus::Malformed,
                    read_only: false,
                };
            }
        };
        if parsed_value
            .get("version")
            .and_then(serde_json::Value::as_u64)
            .is_some_and(|version| version > u64::from(SETTINGS_VERSION))
        {
            return Self {
                path,
                value: Settings::default(),
                status: LoadStatus::UnsupportedFuture,
                read_only: true,
            };
        }
        match serde_json::from_value::<Settings>(parsed_value) {
            Ok(mut value) if value.version == SETTINGS_VERSION => {
                if !shortcut_is_available(&value.command_palette_shortcut) {
                    value.command_palette_shortcut = DEFAULT_PALETTE_SHORTCUT.into();
                }
                Self {
                    path,
                    value,
                    status: LoadStatus::Supported,
                    read_only: false,
                }
            }
            _ => Self {
                path,
                value: Settings::default(),
                status: LoadStatus::Malformed,
                read_only: false,
            },
        }
    }
    pub fn value(&self) -> &Settings {
        &self.value
    }
    pub fn value_mut(&mut self) -> &mut Settings {
        &mut self.value
    }
    pub const fn status(&self) -> LoadStatus {
        self.status
    }
    pub const fn is_read_only(&self) -> bool {
        self.read_only
    }
    pub fn set_palette_shortcut(&mut self, shortcut: &str) -> bool {
        if self.read_only || !shortcut_is_available(shortcut) {
            return false;
        }
        shortcut.clone_into(&mut self.value.command_palette_shortcut);
        true
    }
    pub fn save(&self) -> io::Result<()> {
        if self.read_only {
            return Ok(());
        }
        if !shortcut_is_available(&self.value.command_palette_shortcut) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "command-palette shortcut conflicts with a fixed application shortcut",
            ));
        }
        atomic_save(&self.path, &self.value)
    }
}

/// Rejects the application-wide fixed bindings before a preference reaches disk.
pub fn shortcut_is_available(shortcut: &str) -> bool {
    let normalized = shortcut.trim().to_ascii_lowercase().replace(' ', "");
    let mut parts = normalized.split('+');
    let modifier = parts.next();
    let key = parts.next();
    let has_valid_shape = matches!(modifier, Some("ctrl" | "cmd"))
        && key.is_some_and(|key| !key.is_empty())
        && parts.next().is_none();
    has_valid_shape
        && !matches!(
            normalized.as_str(),
            "ctrl+n"
                | "ctrl+shift+a"
                | "ctrl+i"
                | "ctrl+f"
                | "ctrl+z"
                | "ctrl+shift+z"
                | "ctrl+e"
                | "ctrl+1"
                | "ctrl+2"
                | "ctrl+3"
                | "ctrl+left"
                | "ctrl+right"
                | "ctrl+,"
                | "ctrl+shift+b"
                | "ctrl+\\"
        )
}

fn atomic_save(path: &Path, value: &Settings) -> io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "settings path has no parent")
    })?;
    fs::create_dir_all(parent)?;
    let temp = parent.join(format!("{TEMP_PREFIX}{}", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        serde_json::to_writer_pretty(&mut file, value).map_err(io::Error::other)?;
        file.write_all(b"\n")?;
        file.flush()?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temp, path)?;
        sync_directory(parent)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

fn cleanup_abandoned_temps(path: &Path) {
    let Some(parent) = path.parent() else {
        return;
    };
    let Ok(entries) = fs::read_dir(parent) else {
        return;
    };
    for entry in entries.flatten() {
        if entry.file_name().to_string_lossy().starts_with(TEMP_PREFIX) {
            let _ = fs::remove_file(entry.path());
        }
    }
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> io::Result<()> {
    File::open(path)?.sync_all()
}
#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> io::Result<()> {
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WorkArea {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// Pure monitor geometry used by all platforms and unit-testable without a display server.
pub fn clamp_window(window: WindowBounds, monitors: &[WorkArea]) -> WindowBounds {
    const MIN_W: f32 = 900.0;
    const MIN_H: f32 = 600.0;
    let Some(area) = select_monitor(window, monitors) else {
        return window;
    };
    let width = window.width.max(MIN_W).min(area.width);
    let height = window.height.max(MIN_H).min(area.height);
    WindowBounds {
        x: window.x.max(area.x).min(area.x + area.width - width),
        y: window.y.max(area.y).min(area.y + area.height - height),
        width,
        height,
    }
}

fn select_monitor(window: WindowBounds, monitors: &[WorkArea]) -> Option<WorkArea> {
    let center = (
        window.x + window.width / 2.0,
        window.y + window.height / 2.0,
    );
    monitors
        .iter()
        .copied()
        .min_by(|a, b| distance_squared(center, *a).total_cmp(&distance_squared(center, *b)))
}
fn distance_squared(point: (f32, f32), area: WorkArea) -> f32 {
    let x = point.0.clamp(area.x, area.x + area.width);
    let y = point.1.clamp(area.y, area.y + area.height);
    (point.0 - x).powi(2) + (point.1 - y).powi(2)
}

/// OS monitor discovery is isolated here. An empty result lets the windowing
/// backend choose safely when work areas cannot be queried before its event loop.
pub mod platform {
    use super::WorkArea;
    #[cfg(target_os = "windows")]
    pub fn available_work_areas() -> Vec<WorkArea> {
        // Winit performs the actual Windows monitor selection once its event loop exists.
        Vec::new()
    }
    #[cfg(not(target_os = "windows"))]
    pub fn available_work_areas() -> Vec<WorkArea> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::tempdir;

    #[test]
    fn defaults_are_explicit_and_valid() {
        let s = Settings::default();
        assert_eq!(s.version, 1);
        assert_eq!(s.command_palette_shortcut, "Ctrl+P");
        assert_eq!(s.window, WindowBounds::default());
    }
    #[test]
    fn serialization_is_an_exact_allowlist() {
        let value = serde_json::to_value(Settings::default()).unwrap();
        let mut keys: Vec<_> = value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "command_palette_shortcut",
                "display_density",
                "inbox_thresholds",
                "inspector_visible",
                "last_opened_budget",
                "register_columns",
                "theme",
                "version",
                "window"
            ]
        );
        for forbidden in [
            "sidebar_width",
            "inspector_width",
            "workspace",
            "selected_account",
            "viewed_month",
            "search_history",
            "saved_filters",
        ] {
            assert!(value.get(forbidden).is_none());
        }
    }
    #[test]
    fn round_trip_and_atomic_replacement() {
        let d = tempdir().unwrap();
        let p = d.path().join("settings.json");
        let mut s = SettingsSession::load(&p);
        s.value_mut().theme = Theme::Dark;
        s.save().unwrap();
        let mut second = SettingsSession::load(&p);
        assert_eq!(second.value().theme, Theme::Dark);
        second.value_mut().theme = Theme::Light;
        second.save().unwrap();
        assert_eq!(SettingsSession::load(p).value().theme, Theme::Light);
    }
    #[test]
    fn malformed_supported_json_falls_back() {
        let d = tempdir().unwrap();
        let p = d.path().join("settings.json");
        fs::write(&p, br#"{"version":1,"sidebar_width":20}"#).unwrap();
        let s = SettingsSession::load(p);
        assert_eq!(s.status(), LoadStatus::Malformed);
        assert_eq!(s.value(), &Settings::default());
    }
    #[test]
    fn future_version_is_preserved_byte_for_byte() {
        let d = tempdir().unwrap();
        let p = d.path().join("settings.json");
        let bytes = b"{ \"version\": 999, \"future\": true }\n";
        fs::write(&p, bytes).unwrap();
        let s = SettingsSession::load(&p);
        assert!(s.is_read_only());
        s.save().unwrap();
        assert_eq!(fs::read(p).unwrap(), bytes);
    }
    #[test]
    fn abandoned_write_does_not_touch_valid_file() {
        let d = tempdir().unwrap();
        let p = d.path().join("settings.json");
        fs::write(&p, serde_json::to_vec(&Settings::default()).unwrap()).unwrap();
        let temp = d.path().join(format!("{TEMP_PREFIX}dead"));
        fs::write(&temp, b"partial").unwrap();
        let s = SettingsSession::load(&p);
        assert_eq!(s.status(), LoadStatus::Supported);
        assert!(!temp.exists());
    }
    #[test]
    fn shortcut_conflicts_are_rejected() {
        let d = tempdir().unwrap();
        let mut s = SettingsSession::load(d.path().join("settings.json"));
        assert!(!s.set_palette_shortcut("Ctrl+F"));
        assert!(s.set_palette_shortcut("Ctrl+P"));
        let mut v = serde_json::to_value(Settings::default()).unwrap();
        v["command_palette_shortcut"] = json!("Ctrl+N");
        fs::write(
            d.path().join("settings.json"),
            serde_json::to_vec(&v).unwrap(),
        )
        .unwrap();
        assert_eq!(
            SettingsSession::load(d.path().join("settings.json"))
                .value()
                .command_palette_shortcut,
            "Ctrl+P"
        );
    }
    #[test]
    fn clamps_offscreen_and_selects_nearest_monitor() {
        let monitors = [
            WorkArea {
                x: 0.0,
                y: 0.0,
                width: 1920.0,
                height: 1080.0,
            },
            WorkArea {
                x: 1920.0,
                y: 0.0,
                width: 1280.0,
                height: 720.0,
            },
        ];
        assert_eq!(
            clamp_window(
                WindowBounds {
                    x: 5000.0,
                    y: 900.0,
                    width: 1600.0,
                    height: 900.0
                },
                &monitors
            ),
            WindowBounds {
                x: 1920.0,
                y: 0.0,
                width: 1280.0,
                height: 720.0
            }
        );
        assert!(
            (clamp_window(
                WindowBounds {
                    x: -200.0,
                    y: 10.0,
                    width: 1000.0,
                    height: 700.0
                },
                &monitors
            )
            .x - 0.0)
                .abs()
                < f32::EPSILON
        );
    }
}
