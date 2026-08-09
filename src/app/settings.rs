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

pub const SETTINGS_VERSION: u32 = 3;
pub const DEFAULT_PALETTE_SHORTCUT: &str = "Ctrl+P";
pub const MAX_SAVED_FILTERS: usize = 50;
pub const MAX_SEARCH_HISTORY: usize = 25;
const TEMP_PREFIX: &str = ".settings.json.tmp-";

#[allow(clippy::struct_field_names)]
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
    Compact,
    #[default]
    Normal,
    Comfortable,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct RegisterColumns {
    pub order: Vec<String>,
    pub widths: Vec<f32>,
    /// Hidden columns remain in `order`, making re-enabling deterministic.
    pub hidden: Vec<String>,
}
impl Default for RegisterColumns {
    fn default() -> Self {
        Self {
            order: vec![
                "date", "payee", "category", "memo", "outflow", "inflow", "cleared", "approved",
                "account",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
            widths: vec![90.0, 180.0, 180.0, 200.0, 100.0, 100.0, 80.0, 90.0, 160.0],
            hidden: vec!["account".into()],
        }
    }
}

impl RegisterColumns {
    pub const KNOWN: [&'static str; 9] = [
        "date", "payee", "category", "memo", "outflow", "inflow", "cleared", "approved", "account",
    ];

    #[must_use]
    pub fn minimum_width(column: &str) -> f32 {
        match column {
            "date" => 72.0,
            "payee" | "category" | "account" => 96.0,
            "memo" => 120.0,
            "outflow" | "inflow" => 80.0,
            "cleared" | "approved" => 68.0,
            _ => 40.0,
        }
    }

    /// Repairs untrusted persisted layouts: invalid/duplicate entries are discarded,
    /// missing columns are appended in default order, and unsafe widths are reset.
    pub fn repair(&mut self) {
        let defaults = Self::default();
        let mut repaired = Vec::with_capacity(Self::KNOWN.len());
        let mut widths = Vec::with_capacity(Self::KNOWN.len());
        for (column, width) in self.order.iter().zip(&self.widths) {
            if Self::KNOWN.contains(&column.as_str()) && !repaired.contains(column) {
                repaired.push(column.clone());
                widths.push(
                    if width.is_finite()
                        && *width >= Self::minimum_width(column)
                        && *width <= 1000.0
                    {
                        *width
                    } else {
                        defaults.widths[Self::KNOWN.iter().position(|x| x == column).unwrap()]
                    },
                );
            }
        }
        for (index, column) in Self::KNOWN.iter().enumerate() {
            if !repaired.iter().any(|value| value == column) {
                repaired.push((*column).to_owned());
                widths.push(defaults.widths[index]);
            }
        }
        self.order = repaired;
        self.widths = widths;
        self.hidden
            .retain(|column| Self::KNOWN.contains(&column.as_str()));
        self.hidden.sort();
        self.hidden.dedup();
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn set_width(&mut self, column: &str, width: f32) -> bool {
        let Some(index) = self.order.iter().position(|value| value == column) else {
            return false;
        };
        if !width.is_finite() || width < Self::minimum_width(column) || width > 1000.0 {
            return false;
        }
        self.widths[index] = width;
        true
    }

    pub fn set_visible(&mut self, column: &str, visible: bool) -> bool {
        if !Self::KNOWN.contains(&column) {
            return false;
        }
        self.hidden.retain(|value| value != column);
        if !visible {
            self.hidden.push(column.to_owned());
        }
        true
    }

    #[must_use]
    pub fn visible_for_scope(&self, column: &str, all_transactions: bool) -> bool {
        (column != "account" || all_transactions)
            && !self.hidden.iter().any(|value| value == column)
    }

    pub fn move_column(&mut self, column: &str, destination: usize) -> bool {
        let Some(source) = self.order.iter().position(|value| value == column) else {
            return false;
        };
        if destination >= self.order.len() {
            return false;
        }
        let name = self.order.remove(source);
        let width = self.widths.remove(source);
        self.order.insert(destination, name);
        self.widths.insert(destination, width);
        true
    }
}

/// Only inbox tuning is persisted here; this is not a general saved-filter model.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct InboxThresholds {
    pub duplicate_window_days: u16,
    pub due_soon_days: u16,
    pub uncleared_age_days: u16,
    pub reconciliation_cadence_days: u16,
}
impl Default for InboxThresholds {
    fn default() -> Self {
        Self {
            duplicate_window_days: 7,
            due_soon_days: 7,
            uncleared_age_days: 30,
            reconciliation_cadence_days: 30,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub version: u32,
    pub last_selected_account_id: Option<String>,
    pub last_workspace: Option<String>,
    pub collapsed_account_groups: Vec<String>,
    pub window: WindowBounds,
    pub inspector_visible: bool,
    pub register_columns: RegisterColumns,
    pub theme: Theme,
    pub display_density: DisplayDensity,
    #[serde(skip)]
    pub command_palette_shortcut: String,
    #[serde(skip)]
    pub inbox_thresholds: InboxThresholds,
    #[serde(skip)]
    pub saved_filters: Vec<PersistedFilter>,
    #[serde(skip)]
    pub search_history: Vec<SearchHistoryEntry>,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            version: SETTINGS_VERSION,
            last_selected_account_id: None,
            last_workspace: None,
            collapsed_account_groups: Vec::new(),
            window: WindowBounds::default(),
            inspector_visible: true,
            register_columns: RegisterColumns::default(),
            theme: Theme::default(),
            display_density: DisplayDensity::default(),
            command_palette_shortcut: DEFAULT_PALETTE_SHORTCUT.into(),
            inbox_thresholds: InboxThresholds::default(),
            saved_filters: Vec::new(),
            search_history: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct PersistedFilter {
    pub version: u32,
    pub name: String,
    pub expression: String,
    pub scope: String,
    pub sort_key: String,
    pub descending: bool,
}
impl Default for PersistedFilter {
    fn default() -> Self {
        Self {
            version: 1,
            name: String::new(),
            expression: String::new(),
            scope: "all_accounts".into(),
            sort_key: "date".into(),
            descending: true,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct SearchHistoryEntry {
    pub version: u32,
    pub expression: String,
    pub scope: String,
    pub sort_key: String,
    pub descending: bool,
}
impl Default for SearchHistoryEntry {
    fn default() -> Self {
        Self {
            version: 1,
            expression: String::new(),
            scope: "global".into(),
            sort_key: "relevance".into(),
            descending: true,
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
        let mut parsed_value = parsed_value;
        if let Some(object) = parsed_value.as_object_mut() {
            object.remove("last_opened_budget");
        }
        match serde_json::from_value::<Settings>(parsed_value) {
            Ok(mut value) if value.version <= SETTINGS_VERSION => {
                value.version = SETTINGS_VERSION;
                if !shortcut_is_available(&value.command_palette_shortcut) {
                    value.command_palette_shortcut = DEFAULT_PALETTE_SHORTCUT.into();
                }
                value.register_columns.repair();
                value.repair_filters();
                value.last_selected_account_id = value
                    .last_selected_account_id
                    .take()
                    .filter(|raw| raw.parse::<crate::domain::AccountId>().is_ok());
                value
                    .collapsed_account_groups
                    .retain(|raw| raw.parse::<crate::domain::AccountGroupId>().is_ok());
                if !matches!(
                    value.last_workspace.as_deref(),
                    None | Some(
                        "overview"
                            | "budget"
                            | "all_transactions"
                            | "categories"
                            | "reports"
                            | "inbox"
                            | "account"
                    )
                ) {
                    value.last_workspace = None;
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

impl Settings {
    /// Drop malformed UUIDs and IDs that are not present in the opened database.
    pub fn repair_persisted_ids(
        &mut self,
        valid_accounts: &[crate::domain::AccountId],
        valid_groups: &[crate::domain::AccountGroupId],
    ) {
        self.last_selected_account_id = self
            .last_selected_account_id
            .take()
            .and_then(|raw| raw.parse().ok())
            .filter(|id| valid_accounts.contains(id))
            .map(|id: crate::domain::AccountId| id.to_string());
        self.collapsed_account_groups.retain(|raw| {
            raw.parse::<crate::domain::AccountGroupId>()
                .ok()
                .is_some_and(|id| valid_groups.contains(&id))
        });
        self.collapsed_account_groups.sort();
        self.collapsed_account_groups.dedup();
        if !matches!(
            self.last_workspace.as_deref(),
            None | Some(
                "overview"
                    | "budget"
                    | "all_transactions"
                    | "categories"
                    | "reports"
                    | "inbox"
                    | "account"
            )
        ) {
            self.last_workspace = None;
        }
    }

    pub fn repair_filters(&mut self) {
        let mut warnings = Vec::new();
        self.saved_filters.retain(|f| {
            let ok = f.version == 1
                && !f.name.trim().is_empty()
                && crate::app::search::parse(&f.expression).is_ok();
            if !ok {
                warnings.push(format!("Skipped incompatible saved filter `{}`", f.name));
            }
            ok
        });
        self.saved_filters.sort_by(|a, b| a.name.cmp(&b.name));
        self.saved_filters.dedup_by(|a, b| a.name == b.name);
        if self.saved_filters.len() > MAX_SAVED_FILTERS {
            self.saved_filters.truncate(MAX_SAVED_FILTERS);
        }
        self.search_history
            .retain(|h| h.version == 1 && crate::app::search::parse(&h.expression).is_ok());
        if self.search_history.len() > MAX_SEARCH_HISTORY {
            self.search_history.truncate(MAX_SEARCH_HISTORY);
        }
        if !warnings.is_empty() {
            tracing::warn!("{}", warnings.join("; "));
        }
    }
    pub fn save_filter(&mut self, mut filter: PersistedFilter) -> Result<(), String> {
        if self
            .saved_filters
            .iter()
            .any(|existing| existing.name == filter.name)
        {
            return Err("duplicate saved filter name".into());
        }
        filter.version = 1;
        crate::app::search::parse(&filter.expression).map_err(|_| "invalid filter".to_owned())?;
        self.saved_filters.insert(0, filter);
        if self.saved_filters.len() > MAX_SAVED_FILTERS {
            self.saved_filters.truncate(MAX_SAVED_FILTERS);
        }
        Ok(())
    }
    pub fn rename_filter(&mut self, old: &str, new: &str) -> Result<(), String> {
        if self.saved_filters.iter().any(|f| f.name == new) {
            return Err("duplicate saved filter name".into());
        }
        let f = self
            .saved_filters
            .iter_mut()
            .find(|f| f.name == old)
            .ok_or_else(|| "saved filter not found".to_owned())?;
        f.name = new.into();
        Ok(())
    }
    pub fn delete_filter(&mut self, name: &str) -> bool {
        let old = self.saved_filters.len();
        self.saved_filters.retain(|f| f.name != name);
        old != self.saved_filters.len()
    }
    pub fn reapply_filter(&self, name: &str) -> Option<PersistedFilter> {
        self.saved_filters.iter().find(|f| f.name == name).cloned()
    }
    pub fn remember_search(&mut self, mut entry: SearchHistoryEntry) {
        entry.version = 1;
        if crate::app::search::parse(&entry.expression).is_err() {
            return;
        }
        self.search_history
            .retain(|e| e.expression != entry.expression || e.scope != entry.scope);
        self.search_history.insert(0, entry);
        self.search_history.truncate(MAX_SEARCH_HISTORY);
    }
    pub fn clear_history(&mut self) {
        self.search_history.clear();
    }
}

/// Rejects the application-wide fixed bindings before a preference reaches disk.
pub fn shortcut_is_available(shortcut: &str) -> bool {
    let normalized = shortcut.trim().to_ascii_lowercase().replace(' ', "");
    let mut parts = normalized.split('+');
    let modifier = parts.next();
    let key = parts.next();
    let has_valid_shape = matches!(modifier, Some("ctrl" | "cmd"))
        && key.is_some_and(|key| matches!(key, "p" | "k"))
        && parts.next().is_none();
    has_valid_shape && !crate::ui::keyboard::conflicts_with_fixed(&normalized)
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
        assert_eq!(s.version, SETTINGS_VERSION);
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
                "collapsed_account_groups",
                "display_density",
                "inspector_visible",
                "last_selected_account_id",
                "last_workspace",
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
            "selected_transaction",
            "result_rows",
            "active_drafts",
            "command_queue",
            "confirmations",
            "critical_operation_state",
            "undo_payloads",
            "saved_filters",
            "search_history",
            "inbox_thresholds",
            "command_palette_shortcut",
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
        v["theme"] = json!("dark");
        fs::write(
            d.path().join("settings.json"),
            serde_json::to_vec(&v).unwrap(),
        )
        .unwrap();
        assert_eq!(
            SettingsSession::load(d.path().join("settings.json"))
                .value()
                .theme,
            Theme::Dark
        );
    }
    #[test]
    fn saved_filters_and_history_are_bounded_and_privacy_conscious() {
        let mut s = Settings::default();
        s.save_filter(PersistedFilter {
            name: "Coffee".into(),
            expression: "payee:Coffee".into(),
            ..Default::default()
        })
        .unwrap();
        assert!(
            s.save_filter(PersistedFilter {
                name: "Coffee".into(),
                expression: "memo:beans".into(),
                ..Default::default()
            })
            .is_err()
        );
        s.rename_filter("Coffee", "Cafe").unwrap();
        assert_eq!(s.reapply_filter("Cafe").unwrap().expression, "payee:Coffee");
        assert!(s.delete_filter("Cafe"));
        for index in 0..(MAX_SEARCH_HISTORY + 5) {
            s.remember_search(SearchHistoryEntry {
                expression: format!("memo:item{index}"),
                ..Default::default()
            });
        }
        assert_eq!(s.search_history.len(), MAX_SEARCH_HISTORY);
        let json = serde_json::to_value(&s).unwrap();
        assert!(json.get("result_rows").is_none());
        assert!(json.get("selected_transaction").is_none());
        s.clear_history();
        assert!(s.search_history.is_empty());
    }

    #[test]
    fn malformed_saved_filters_are_skipped_on_repair() {
        let mut s = Settings {
            saved_filters: vec![
                PersistedFilter {
                    name: "Valid".into(),
                    expression: "approved:false".into(),
                    ..Default::default()
                },
                PersistedFilter {
                    version: 999,
                    name: "Future".into(),
                    expression: "approved:false".into(),
                    ..Default::default()
                },
                PersistedFilter {
                    name: "Broken".into(),
                    expression: "before:nope".into(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        s.repair_filters();
        assert_eq!(s.saved_filters.len(), 1);
        assert_eq!(s.saved_filters[0].name, "Valid");
    }
    #[test]
    fn legacy_last_opened_budget_is_ignored_for_compatibility() {
        let d = tempdir().unwrap();
        let p = d.path().join("settings.json");
        std::fs::write(
            &p,
            serde_json::to_vec_pretty(&json!({
                "version": 2,
                "last_opened_budget": "/tmp/old.sqlite3",
                "window": WindowBounds::default(),
                "inspector_visible": true,
                "register_columns": RegisterColumns::default(),
                "theme": "system",
                "display_density": "normal"
            }))
            .unwrap(),
        )
        .unwrap();
        let loaded = SettingsSession::load(&p);
        assert_eq!(loaded.status(), LoadStatus::Supported);
        assert_eq!(loaded.value().version, SETTINGS_VERSION);
        assert!(loaded.value().last_selected_account_id.is_none());
    }

    #[test]
    fn theme_density_round_trip_and_column_repair() {
        let d = tempdir().unwrap();
        let p = d.path().join("settings.json");
        let mut session = SettingsSession::load(&p);
        session.value_mut().theme = Theme::Dark;
        session.value_mut().display_density = DisplayDensity::Compact;
        session.value_mut().register_columns = RegisterColumns {
            order: vec!["memo".into(), "unknown".into(), "memo".into()],
            widths: vec![222.0, 1.0, 333.0],
            hidden: vec!["unknown".into(), "account".into(), "account".into()],
        };
        session.save().unwrap();
        let loaded = SettingsSession::load(p);
        assert_eq!(
            (loaded.value().theme, loaded.value().display_density),
            (Theme::Dark, DisplayDensity::Compact)
        );
        assert_eq!(
            loaded.value().register_columns.order.len(),
            RegisterColumns::KNOWN.len()
        );
        assert_eq!(loaded.value().register_columns.order[0], "memo");
        assert_eq!(loaded.value().register_columns.widths[0], 222.0);
        assert_eq!(loaded.value().register_columns.hidden, vec!["account"]);
    }
    #[test]
    fn register_columns_enforce_typed_minimums_visibility_and_reset() {
        let mut columns = RegisterColumns {
            order: vec!["memo".into(), "date".into()],
            widths: vec![1.0, f32::NAN],
            hidden: vec!["memo".into(), "memo".into(), "unknown".into()],
        };
        columns.repair();
        assert!(columns.widths.iter().zip(&columns.order).all(
            |(width, name)| width.is_finite() && *width >= RegisterColumns::minimum_width(name)
        ));
        assert!(!columns.visible_for_scope("memo", true));
        assert!(!columns.visible_for_scope("account", false));
        assert!(columns.set_visible("memo", true));
        assert!(columns.visible_for_scope("memo", true));
        columns.reset();
        assert_eq!(columns, RegisterColumns::default());
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
