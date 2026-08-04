pub mod budget_catalog;
pub mod command;
pub mod dispatcher;
pub mod inbox;
pub mod lifecycle;
pub mod message;
pub mod navigation;
pub mod palette;
pub mod portable_paths;
pub mod runtime;
pub mod search;
pub mod session;
pub mod settings;
pub mod startup;
pub mod state;
pub mod view_invalidation;
pub mod view_model;

use crate::{
    app::runtime::ApplicationRuntime, error::UserFacingError, storage::LATEST_SCHEMA_VERSION,
};
use portable_paths::PortablePaths;
use settings::{LoadStatus, SettingsSession};
use startup::StartupContext;

pub struct MnabApp {
    startup_error: Option<crate::error::StartupError>,
    runtime: ApplicationRuntime,
}

impl MnabApp {
    pub fn ready(paths: PortablePaths, settings: SettingsSession, startup: StartupContext) -> Self {
        let malformed = matches!(settings.status(), LoadStatus::Malformed);
        Self {
            startup_error: None,
            runtime: ApplicationRuntime::new(Some(paths), Some(settings), malformed, startup),
        }
    }

    pub fn fatal(error: crate::error::StartupError) -> Self {
        Self {
            startup_error: Some(error),
            runtime: ApplicationRuntime::new(
                None,
                None,
                false,
                StartupContext {
                    marker_was_absent: false,
                    last_successfully_opened_budget: None,
                },
            ),
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
                        self.runtime.request_exit();
                    }
                });
            });
            return;
        }
        let (theme, density) = self.runtime.presentation();
        match theme {
            settings::Theme::Light => ctx.set_visuals(egui::Visuals::light()),
            settings::Theme::Dark => ctx.set_visuals(egui::Visuals::dark()),
            settings::Theme::System => {}
        }
        let mut style = (*ctx.style()).clone();
        style.spacing.item_spacing = match density {
            settings::DisplayDensity::Compact => egui::vec2(6.0, 3.0),
            settings::DisplayDensity::Normal => egui::vec2(8.0, 6.0),
            settings::DisplayDensity::Comfortable => egui::vec2(10.0, 9.0),
        };
        ctx.set_style(style);
        if let Some(rect) = ctx.input(|input| input.viewport().outer_rect) {
            self.runtime.record_window(rect);
        }
        if ctx.input(|input| input.viewport().close_requested()) {
            self.runtime.native_close_requested();
        }
        // Responses are always exhausted before input or rendering.  This makes the
        // immutable view snapshot used by this frame internally consistent.
        self.runtime.drain_worker_responses();
        let mut actions = crate::app::dispatcher::ActionCollector::default();
        if self.runtime.lifecycle_state() == lifecycle::LifecycleState::ShuttingDown {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.disable();
                ui.centered_and_justified(|ui| {
                    ui.heading("Closing MNAB…");
                });
            });
        } else {
            crate::ui::shell::show(ctx, self.runtime.view_mut(), &mut actions);
            self.runtime.dispatch_collected(actions);
        }
        if self.runtime.lifecycle_state() == lifecycle::LifecycleState::ShuttingDown {
            let _ = self.runtime.shutdown();
        }
        for effect in self.runtime.take_lifecycle_effects() {
            match effect {
                lifecycle::LifecycleEffect::CancelNativeClose => {
                    ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose)
                }
                lifecycle::LifecycleEffect::SendProgrammaticClose => {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close)
                }
                _ => {}
            }
        }
        if self.runtime.has_pending_work() || ctx.has_requested_repaint() {
            ctx.request_repaint();
        }
        #[cfg(debug_assertions)]
        self.diagnostics(ctx);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if let Err(error) = self.runtime.shutdown() {
            tracing::error!(%error, "shutdown incomplete");
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
                ui.label(format!("Supported schema: {LATEST_SCHEMA_VERSION}"));
                ui.label(format!(
                    "Active schema: {}",
                    self.runtime
                        .session()
                        .map_or_else(|| "none".into(), |s| s.schema_version.to_string())
                ));
            });
    }
}
