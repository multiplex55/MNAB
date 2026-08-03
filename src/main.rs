#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]
#![allow(dead_code)] // Initial architecture includes boundaries used by upcoming features.

mod app;
mod calculation;
mod domain;
mod error;
mod importing;
mod service;
mod storage;
mod ui;

use std::fs;

use app::{
    MnabApp,
    portable_paths::PortablePaths,
    settings::{SettingsSession, WindowBounds, clamp_window, platform},
    startup::StartupContext,
};
use tracing_subscriber::prelude::*;

const TITLE: &str = "MNAB — Multi Needs A Budget";

#[allow(clippy::unnecessary_wraps)]
fn init_logging(
    paths: &PortablePaths,
) -> Result<tracing_appender::non_blocking::WorkerGuard, std::io::Error> {
    let appender = tracing_appender::rolling::daily(&paths.logs, "mnab.log");
    let (writer, guard) = tracing_appender::non_blocking(appender);
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_writer(writer),
        )
        .try_init()
        .ok();
    Ok(guard)
}

fn main() {
    let startup = PortablePaths::discover();
    let mut guard = None;
    let (app, window) = match startup {
        Ok(paths) => {
            match init_logging(&paths) {
                Ok(value) => guard = Some(value),
                Err(source) => {
                    let error = error::StartupError::Logging {
                        path: paths.logs.clone(),
                        source,
                    };
                    return launch(MnabApp::fatal(error), WindowBounds::default(), guard);
                }
            }
            let clean_marker = paths.data.join(".clean-shutdown");
            tracing::info!(version = env!("CARGO_PKG_VERSION"), profile = if cfg!(debug_assertions) { "debug" } else { "release" }, executable = %paths.executable.display(), data = %paths.data.display(), "starting MNAB");
            // Settings must be read before any native window options are built.
            let settings = SettingsSession::load(&paths.settings);
            let startup = StartupContext::capture(&clean_marker, &settings);
            if startup.marker_was_absent {
                tracing::warn!("previous session did not record a clean shutdown");
            }
            if let Err(error) = fs::remove_file(&clean_marker)
                && error.kind() != std::io::ErrorKind::NotFound
            {
                tracing::error!(%error, "could not consume clean marker");
            }
            let window = clamp_window(settings.value().window, &platform::available_work_areas());
            (MnabApp::ready(paths, settings, startup), window)
        }
        Err(error) => {
            eprintln!("MNAB startup failure: {error:#}");
            (MnabApp::fatal(error), WindowBounds::default())
        }
    };
    launch(app, window, guard);
}

fn launch(
    app: MnabApp,
    window: WindowBounds,
    guard: Option<tracing_appender::non_blocking::WorkerGuard>,
) {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(TITLE)
            .with_position([window.x, window.y])
            .with_inner_size([window.width, window.height])
            .with_min_inner_size([900.0, 600.0])
            .with_icon(std::sync::Arc::new(application_icon())),
        ..Default::default()
    };
    let result = eframe::run_native(TITLE, options, Box::new(move |_| Ok(Box::new(app))));
    if let Err(error) = &result {
        tracing::error!(error = %error, "window initialization failed; shutdown is unclean");
        present_window_failure(&error.to_string());
    }
    drop(guard);
}

fn application_icon() -> egui::IconData {
    const SIDE: u32 = 32;
    let mut rgba = Vec::with_capacity((SIDE * SIDE * 4) as usize);
    for y in 0..SIDE {
        for x in 0..SIDE {
            let accent = (6..26).contains(&x) && (6..26).contains(&y);
            rgba.extend_from_slice(if accent {
                &[42, 157, 143, 255]
            } else {
                &[30, 45, 62, 255]
            });
        }
    }
    egui::IconData {
        rgba,
        width: SIDE,
        height: SIDE,
    }
}

fn present_window_failure(detail: &str) {
    #[cfg(target_os = "windows")]
    {
        rfd::MessageDialog::new()
            .set_title("MNAB could not start")
            .set_description(
                "The MNAB window could not be initialized. See mnab-data/logs for details.",
            )
            .set_level(rfd::MessageLevel::Error)
            .show();
    }
    #[cfg(not(target_os = "windows"))]
    eprintln!("MNAB window initialization failed: {detail}");
    #[cfg(all(target_os = "windows", debug_assertions))]
    eprintln!("MNAB window initialization failed: {detail}");
}
