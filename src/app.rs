pub mod command;
pub mod message;
pub mod navigation;
pub mod portable_paths;
pub mod state;

use std::path::PathBuf;

use crate::{
    app::{command::AppCommand, state::AppState},
    error::UserFacingError,
};
use portable_paths::PortablePaths;

pub const SUPPORTED_SCHEMA_VERSION: u32 = 1;

pub struct MnabApp {
    paths: Option<PortablePaths>,
    startup_error: Option<crate::error::StartupError>,
    commands: Vec<AppCommand>,
    active_database: Option<PathBuf>,
    state: AppState,
}

impl MnabApp {
    pub fn ready(paths: PortablePaths) -> Self {
        Self {
            paths: Some(paths),
            startup_error: None,
            commands: Vec::new(),
            active_database: None,
            state: AppState::default(),
        }
    }

    pub fn fatal(error: crate::error::StartupError) -> Self {
        Self {
            paths: None,
            startup_error: Some(error),
            commands: Vec::new(),
            active_database: None,
            state: AppState::default(),
        }
    }

    pub fn paths_for_shutdown(&self) -> Option<&std::path::Path> {
        self.paths.as_ref().map(|paths| paths.data.as_path())
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
        crate::ui::shell::show(ctx, &mut self.state, &mut self.commands);
        #[cfg(debug_assertions)]
        self.diagnostics(ctx);
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
                if let Some(paths) = &self.paths {
                    ui.label(format!("Executable: {}", paths.executable.display()));
                    ui.label(format!("Portable data: {}", paths.data.display()));
                }
                ui.label(format!(
                    "Active database: {}",
                    self.active_database
                        .as_ref()
                        .map_or_else(|| "none".into(), |p| p.display().to_string())
                ));
                ui.label(format!("Supported schema: {SUPPORTED_SCHEMA_VERSION}"));
                ui.label("Active schema: none");
            });
    }
}
