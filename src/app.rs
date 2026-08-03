pub mod command;
pub mod dispatcher;
pub mod message;
pub mod navigation;
pub mod palette;
pub mod portable_paths;
pub mod runtime;
pub mod search;
pub mod session;
pub mod settings;
pub mod state;
pub mod view_invalidation;

use crate::{app::runtime::ApplicationRuntime, error::UserFacingError};
use portable_paths::PortablePaths;
use settings::{LoadStatus, SettingsSession};

pub const SUPPORTED_SCHEMA_VERSION: u32 = 1;

pub struct MnabApp {
    startup_error: Option<crate::error::StartupError>,
    runtime: ApplicationRuntime,
}

impl MnabApp {
    pub fn ready(paths: PortablePaths, settings: SettingsSession) -> Self {
        let malformed = matches!(settings.status(), LoadStatus::Malformed);
        Self {
            startup_error: None,
            runtime: ApplicationRuntime::new(Some(paths), Some(settings), malformed),
        }
    }

    pub fn fatal(error: crate::error::StartupError) -> Self {
        Self {
            startup_error: Some(error),
            runtime: ApplicationRuntime::new(None, None, false),
        }
    }

    pub fn paths_for_shutdown(&self) -> Option<&std::path::Path> {
        self.runtime.paths().map(|paths| paths.data.as_path())
    }
}

impl eframe::App for MnabApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if let Some(error) = &self.startup_error {
            egui::CentralPanel::default().show(ctx, |ui| {
                egui::Modal::new(egui::Id::new("fatal-startup")).show(ui.ctx(), |ui| {
                    ui.heading("MNAB could not start");
                    ui.label(error.user_summary());
                    if let Some(path) = error.affected_path() {
                        ui.label(format!("Affected path: {}", path.display()));
                    }
                    ui.label("Logs: mnab-data/logs beside the application (when writable)");
                    if ui.button("Exit").clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
            });
            return;
        }
        if let Some(rect) = ctx.input(|input| input.viewport().outer_rect) {
            self.runtime.record_window(rect);
        }
        // Responses are always exhausted before input or rendering.  This makes the
        // immutable view snapshot used by this frame internally consistent.
        self.runtime.drain_worker_responses();
        let mut actions = crate::app::dispatcher::ActionCollector::default();
        crate::ui::shell::show(ctx, self.runtime.view_mut(), &mut actions);
        self.runtime.dispatch_collected(actions);
        if self.runtime.has_pending_work() || ctx.has_requested_repaint() {
            ctx.request_repaint();
        }
        #[cfg(debug_assertions)]
        self.diagnostics(ctx);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if let Err(error) = self.runtime.save_settings() {
            tracing::error!(%error, "could not save settings during shutdown");
        }
    }
}

#[cfg(debug_assertions)]
impl MnabApp {
    fn diagnostics(&self, ctx: &egui::Context) {
        egui::Window::new("Development diagnostics")
            .default_open(false)
            .show(ctx, |ui| {
                ui.label(format!("Version: {}", env!("CARGO_PKG_VERSION")));
                ui.label("Build profile: debug");
                if let Some(paths) = self.runtime.paths() {
                    ui.label(format!("Executable: {}", paths.executable.display()));
                    ui.label(format!("Portable data: {}", paths.data.display()));
                }
                ui.label(format!(
                    "Active database: {}",
                    self.runtime
                        .session()
                        .map(|session| &session.database_path)
                        .map_or_else(|| "none".into(), |p| p.display().to_string())
                ));
                ui.label(format!("Supported schema: {SUPPORTED_SCHEMA_VERSION}"));
                ui.label(format!(
                    "Active schema: {}",
                    self.runtime
                        .session()
                        .map_or_else(|| "none".into(), |s| s.schema_version.to_string())
                ));
            });
    }
}
