use std::collections::BTreeMap;

use crate::{
    app::{
        command::{
            ApplicationAction, CommandEnvelope, CommandHistory, FinancialCommand, HistoryEntry,
        },
        dispatcher::{ActionCollector, validate_confirmation},
        portable_paths::PortablePaths,
        session::BudgetSession,
        settings::SettingsSession,
        startup::StartupContext,
        state::{AppState, Notification, NotificationKind},
        view_invalidation::ViewInvalidations,
    },
    storage::worker::{Generation, RequestId, StorageResponse, StorageWorker},
};

/// Owns all mutable application-lifetime and active-session machinery. The UI
/// receives only `AppState` (its view model) and a frame-local action sink.
pub struct ApplicationRuntime {
    paths: Option<PortablePaths>,
    session: Option<BudgetSession>,
    worker: Option<StorageWorker>,
    next_request: RequestId,
    next_command: u64,
    generation: Generation,
    view: AppState,
    history: CommandHistory<FinancialCommand>,
    invalidations: ViewInvalidations,
    pending_commands: BTreeMap<u64, CommandEnvelope>,
    settings: Option<SettingsSession>,
    accepting_commands: bool,
    shutdown_complete: bool,
    shutdown_requested: bool,
}

impl ApplicationRuntime {
    /// Deletion is non-financial lifecycle work and is intentionally never recorded
    /// in command history. The active worker is closed (and checkpoints WAL) first.
    pub fn delete_budget(
        &mut self,
        catalog: &mut crate::app::budget_catalog::BudgetCatalog,
        id: crate::domain::BudgetId,
        confirmation: &str,
    ) -> Result<crate::app::budget_catalog::DeletionResult, crate::app::budget_catalog::CatalogError>
    {
        catalog.confirm_name(id, confirmation)?;
        let is_active = self.session.as_ref().is_some_and(|s| s.budget_id == id);
        if is_active {
            self.close_session();
        }
        let paths = self
            .paths
            .as_ref()
            .ok_or(crate::app::budget_catalog::CatalogError::UnmanagedPath)?;
        catalog.delete(paths, id, confirmation)
    }
    pub fn new(
        paths: Option<PortablePaths>,
        settings: Option<SettingsSession>,
        malformed_settings: bool,
        startup: StartupContext,
    ) -> Self {
        let mut view = AppState::default();
        if let Some(settings) = &settings {
            view.inspector_visible = settings.value().inspector_visible;
        }
        if malformed_settings {
            view.notifications.push(Notification {
                kind: NotificationKind::Warning,
                title: "Settings were reset".into(),
                detail: "settings.json is malformed or contains invalid supported-version data; defaults are in use.".into(),
                persistent: true,
            });
        }
        let mut runtime = Self {
            paths,
            session: None,
            worker: None,
            next_request: 1,
            next_command: 1,
            generation: Generation { budget: 0, view: 0 },
            view,
            history: CommandHistory::new(100),
            invalidations: ViewInvalidations::default(),
            pending_commands: BTreeMap::new(),
            settings,
            accepting_commands: true,
            shutdown_complete: false,
            shutdown_requested: false,
        };
        runtime.apply_startup(startup);
        runtime
    }

    fn apply_startup(&mut self, startup: StartupContext) {
        let Some(selected) = startup.last_successfully_opened_budget else {
            if startup.marker_was_absent {
                self.startup_notice(NotificationKind::Warning, "Startup checks pending", "The previous shutdown was unclean; select a budget to run complete diagnostics before opening.");
            }
            return;
        };
        let Some(paths) = self.paths.clone() else {
            return;
        };
        let result = crate::app::budget_catalog::BudgetCatalog::default().prepare_open_checked(
            &paths,
            &selected,
            startup.marker_was_absent,
            || {},
        );
        match result {
            Ok(prepared) => {
                self.commit_session(prepared.session, prepared.worker);
                if startup.marker_was_absent {
                    self.startup_notice(NotificationKind::Information, "Startup diagnostics passed", "Integrity, foreign-key, and financial diagnostics passed after the unclean shutdown.");
                }
            }
            Err(error) => {
                let title = if startup.marker_was_absent {
                    "Startup diagnostics failed"
                } else {
                    "Last budget was not opened"
                };
                self.startup_notice(
                    NotificationKind::Error,
                    title,
                    &format!("Normal opening was refused: {error}"),
                );
            }
        }
    }
    fn startup_notice(&mut self, kind: NotificationKind, title: &str, detail: &str) {
        self.view.notifications.push(Notification {
            kind,
            title: title.into(),
            detail: detail.into(),
            persistent: true,
        });
    }
    pub fn save_settings(&mut self) -> std::io::Result<()> {
        if let Some(settings) = &mut self.settings {
            settings.value_mut().inspector_visible = self.view.inspector_visible;
            if let Some(session) = &self.session {
                settings.value_mut().last_opened_budget = Some(session.database_path.clone());
            }
            settings.save()?;
        }
        Ok(())
    }
    pub fn record_window(&mut self, rect: egui::Rect) {
        if let Some(settings) = &mut self.settings {
            settings.value_mut().window = crate::app::settings::WindowBounds {
                x: rect.min.x,
                y: rect.min.y,
                width: rect.width(),
                height: rect.height(),
            };
        }
    }
    pub fn paths(&self) -> Option<&PortablePaths> {
        self.paths.as_ref()
    }
    pub fn session(&self) -> Option<&BudgetSession> {
        self.session.as_ref()
    }
    pub fn view_mut(&mut self) -> &mut AppState {
        &mut self.view
    }
    pub fn generation(&self) -> Generation {
        self.generation
    }
    pub fn allocate_request(&mut self) -> RequestId {
        let id = self.next_request;
        self.next_request = self
            .next_request
            .checked_add(1)
            .expect("request id exhausted");
        id
    }
    pub fn bump_view_generation(&mut self) {
        self.generation.view = self
            .generation
            .view
            .checked_add(1)
            .expect("view generation exhausted");
        self.view.generation = self.generation;
    }
    pub fn commit_session(&mut self, session: BudgetSession, worker: StorageWorker) {
        // Callers fully initialize the candidate before committing it. Thus the
        // old worker remains usable if candidate preparation fails.
        if let Some(mut old) = self.worker.take() {
            let _ = old.shutdown();
        }
        let budget_id = session.budget_id;
        let database_path = session.database_path.clone();
        let budget_name = session.summary.budget_name.clone();
        self.session = Some(session);
        self.worker = Some(worker);
        self.generation.budget = self
            .generation
            .budget
            .checked_add(1)
            .expect("budget generation exhausted");
        self.generation.view = 0;
        self.view.generation = self.generation;
        self.history.clear();
        self.pending_commands.clear();
        self.view.clear_budget_state();
        self.view.active_budget = Some(budget_id);
        self.view.database_path = Some(database_path);
        self.view.budget_name = budget_name;
    }
    pub fn close_session(&mut self) {
        if let Some(mut worker) = self.worker.take() {
            let _ = worker.shutdown();
        }
        self.session = None;
        self.generation.budget = self.generation.budget.saturating_add(1);
        self.generation.view = 0;
        self.view.generation = self.generation;
        self.history.clear();
        self.pending_commands.clear();
        self.view.clear_budget_state();
    }
    pub fn drain_worker_responses(&mut self) {
        let ready = self
            .worker
            .as_ref()
            .map_or_else(Vec::new, StorageWorker::drain_ready);
        for response in ready {
            match response {
                StorageResponse::Completed {
                    id,
                    generation,
                    result,
                    ..
                } if generation == self.generation => {
                    self.view.complete_request(id);
                    if let Err(error) = result {
                        tracing::error!(request_id=id, error=?error, "storage command failed");
                        self.view.notifications.push(Notification {
                            kind: NotificationKind::Error,
                            title: "Operation failed".into(),
                            detail:
                                "MNAB could not complete that operation. Your data was not changed."
                                    .into(),
                            persistent: true,
                        });
                    }
                }
                StorageResponse::Completed { id, .. } => {
                    tracing::debug!(request_id = id, "discarded stale worker response");
                }
                StorageResponse::Terminated => {
                    self.worker = None;
                    break;
                }
                StorageResponse::Progress { id, generation, .. }
                    if generation == self.generation =>
                {
                    tracing::trace!(request_id = id, "storage operation progressed");
                }
                StorageResponse::Progress { id, .. } => {
                    tracing::debug!(request_id = id, "discarded stale worker progress");
                }
            }
        }
    }
    pub fn dispatch_collected(&mut self, actions: ActionCollector) {
        for action in actions.into_actions() {
            self.dispatch(action);
        }
    }
    fn dispatch(&mut self, action: ApplicationAction) {
        if !self.accepting_commands {
            return;
        }
        if let ApplicationAction::Ui(intent) = action {
            // Global intentions are handled by the application router and are not
            // mistaken for persistence work.
            if intent == crate::app::command::AppCommand::ToggleInspector {
                self.view.inspector_visible = !self.view.inspector_visible;
            }
            if intent == crate::app::command::AppCommand::Exit {
                self.shutdown_requested = true;
            }
            return;
        }
        let id = self.next_command;
        self.next_command = self.next_command.saturating_add(1);
        let envelope = CommandEnvelope {
            command_id: id,
            correlation_id: id,
            budget_generation: self.generation.budget,
            payload: action,
            confirmation_token: None,
            focus_restoration_id: None,
        };
        if validate_confirmation(&envelope).is_err() {
            self.view.notifications.push(Notification {
                kind: NotificationKind::Warning,
                title: "Confirmation required".into(),
                detail: "Review and confirm this change before continuing.".into(),
                persistent: true,
            });
            return;
        }
        self.pending_commands.insert(id, envelope);
    }
    pub fn record_command_success(
        &mut self,
        entry: HistoryEntry<FinancialCommand>,
        invalidations: ViewInvalidations,
    ) {
        self.history.record_success(entry);
        self.invalidations.merge(invalidations);
    }
    pub fn has_pending_work(&self) -> bool {
        !self.pending_commands.is_empty() || !self.view.operations.is_empty()
    }
    pub fn history(&self) -> &CommandHistory<FinancialCommand> {
        &self.history
    }
    pub const fn shutdown_requested(&self) -> bool {
        self.shutdown_requested
    }
    pub fn request_shutdown(&mut self) {
        self.shutdown_requested = true;
    }

    /// The sole ordered shutdown state machine. A marker is published only after every
    /// required durability step has succeeded.
    pub fn shutdown(&mut self) -> Result<(), String> {
        if self.shutdown_complete {
            return Ok(());
        }
        self.accepting_commands = false;
        self.pending_commands.clear(); // queued mutations have not started and are rolled back
        if !self.view.operations.is_empty() {
            self.view.operations.clear();
        }
        if let Some(mut worker) = self.worker.take() {
            worker
                .shutdown()
                .map_err(|e| Self::shutdown_failed("worker stop/join and WAL checkpoint", e))?;
        }
        self.save_settings()
            .map_err(|e| Self::shutdown_failed("settings persistence", e))?;
        let paths = self
            .paths
            .as_ref()
            .ok_or_else(|| Self::shutdown_failed("clean marker", "portable paths unavailable"))?;
        write_clean_marker(&paths.data.join(".clean-shutdown"))
            .map_err(|e| Self::shutdown_failed("clean marker write/sync", e))?;
        self.shutdown_complete = true;
        Ok(())
    }
    fn shutdown_failed(step: &str, error: impl std::fmt::Display) -> String {
        let message = format!("shutdown step '{step}' failed: {error}");
        tracing::error!(step, error=%error, "required shutdown step failed; clean marker omitted");
        message
    }
}

fn write_clean_marker(path: &std::path::Path) -> std::io::Result<()> {
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)?;
    file.write_all(b"clean")?;
    file.sync_all()?;
    if let Some(parent) = path.parent() {
        sync_directory(parent)?;
    }
    Ok(())
}

// Unix permits opening a directory and syncing its metadata. Windows denies
// opening a directory through `File::open`; the marker file itself has already
// been flushed above, so do not turn a clean Windows shutdown into a failure.
#[cfg(unix)]
fn sync_directory(path: &std::path::Path) -> std::io::Result<()> {
    std::fs::File::open(path)?.sync_all()
}
#[cfg(not(unix))]
fn sync_directory(_path: &std::path::Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn request_ids_are_monotonic_and_generations_are_independent() {
        let mut runtime = ApplicationRuntime::new(
            None,
            None,
            false,
            StartupContext {
                marker_was_absent: false,
                last_successfully_opened_budget: None,
            },
        );
        assert_eq!(
            (runtime.allocate_request(), runtime.allocate_request()),
            (1, 2)
        );
        let before = runtime.generation();
        runtime.bump_view_generation();
        assert_eq!(runtime.generation().budget, before.budget);
        assert_eq!(runtime.generation().view, before.view + 1);
    }

    #[test]
    fn invalid_replacement_leaves_old_session_worker_usable() {
        use crate::{
            app::budget_catalog::BudgetCatalog,
            domain::BudgetId,
            storage::worker::{SessionOperation, StorageRequest, WorkerOperation},
        };
        let dir = tempfile::tempdir().unwrap();
        let paths = PortablePaths::from_executable(&dir.path().join("mnab.exe")).unwrap();
        let database = paths.budgets.join("old.sqlite3");
        let worker = StorageWorker::start(&database, || {}).unwrap();
        let mut runtime = ApplicationRuntime::new(
            Some(paths.clone()),
            None,
            false,
            StartupContext {
                marker_was_absent: false,
                last_successfully_opened_budget: None,
            },
        );
        runtime.commit_session(
            BudgetSession {
                budget_id: BudgetId::new(),
                database_path: database,
                schema_version: 3,
                summary: crate::app::session::SessionSummary {
                    budget_name: "Old".into(),
                    account_count: 0,
                },
            },
            worker,
        );
        let invalid = paths.budgets.join("invalid.sqlite3");
        std::fs::write(&invalid, b"not sqlite").unwrap();
        assert!(
            BudgetCatalog::default()
                .prepare_open(&paths, &invalid, || {})
                .is_err()
        );
        runtime
            .worker
            .as_ref()
            .unwrap()
            .submit(StorageRequest {
                id: 1,
                generation: runtime.generation,
                operation: WorkerOperation::Session(SessionOperation::Health),
            })
            .unwrap();
        assert!(
            runtime
                .worker
                .as_ref()
                .unwrap()
                .response_timeout(std::time::Duration::from_secs(2))
                .is_some()
        );
        assert_eq!(runtime.session().unwrap().summary.budget_name, "Old");
    }

    #[test]
    fn prior_generation_response_cannot_complete_new_session_operation() {
        let mut runtime = ApplicationRuntime::new(
            None,
            None,
            false,
            StartupContext {
                marker_was_absent: false,
                last_successfully_opened_budget: None,
            },
        );
        runtime.generation = Generation { budget: 2, view: 0 };
        runtime.view.generation = runtime.generation;
        let stale = StorageResponse::Completed {
            id: 7,
            generation: Generation { budget: 1, view: 0 },
            result: Ok(crate::storage::worker::TypedResult::Healthy),
            invalidations: None,
            user_error: None,
            diagnostic: None,
        };
        assert!(!crate::storage::worker::response_is_current(
            &stale,
            7,
            runtime.generation
        ));
    }

    #[test]
    fn command_exit_and_window_close_use_same_idempotent_shutdown() {
        let dir = tempfile::tempdir().unwrap();
        let paths = PortablePaths::from_executable(&dir.path().join("mnab.exe")).unwrap();
        let settings = SettingsSession::load(&paths.settings);
        let context = StartupContext {
            marker_was_absent: false,
            last_successfully_opened_budget: None,
        };
        let mut command =
            ApplicationRuntime::new(Some(paths.clone()), Some(settings), false, context.clone());
        command.dispatch(ApplicationAction::Ui(crate::app::command::AppCommand::Exit));
        assert!(command.shutdown_requested());
        command.shutdown().unwrap();
        command.shutdown().unwrap();
        assert!(paths.data.join(".clean-shutdown").is_file());

        std::fs::remove_file(paths.data.join(".clean-shutdown")).unwrap();
        let mut window = ApplicationRuntime::new(
            Some(paths.clone()),
            Some(SettingsSession::load(&paths.settings)),
            false,
            context,
        );
        window.request_shutdown();
        assert_eq!(window.shutdown_requested(), command.shutdown_requested());
        window.shutdown().unwrap();
        assert!(paths.data.join(".clean-shutdown").is_file());
    }

    #[test]
    fn settings_failure_omits_clean_marker() {
        let dir = tempfile::tempdir().unwrap();
        let paths = PortablePaths::from_executable(&dir.path().join("mnab.exe")).unwrap();
        std::fs::create_dir(&paths.settings).unwrap();
        let settings = SettingsSession::load(&paths.settings);
        let mut runtime = ApplicationRuntime::new(
            Some(paths.clone()),
            Some(settings),
            false,
            StartupContext {
                marker_was_absent: false,
                last_successfully_opened_budget: None,
            },
        );
        assert!(runtime.shutdown().is_err());
        assert!(!paths.data.join(".clean-shutdown").exists());
    }
}
