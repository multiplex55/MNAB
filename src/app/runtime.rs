use std::collections::BTreeMap;

use crate::{
    app::{
        command::{
            ApplicationAction, CancellationPolicy, CommandEnvelope, CommandHistory, CommandStatus,
            ConfirmationState, DeduplicationKey, FailureSafety, FinancialCommand, HistoryEntry,
            OperationClass, RetryMetadata, Reversibility, RuntimeCommand,
        },
        dispatcher::{ActionCollector, requires_confirmation},
        lifecycle::{Lifecycle, LifecycleEffect, LifecycleState},
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
    pending_commands: BTreeMap<u64, RuntimeCommand>,
    terminal_sequence: u64,
    terminal_capacity: usize,
    settings: Option<SettingsSession>,
    accepting_commands: bool,
    lifecycle: Lifecycle,
    lifecycle_effects: Vec<LifecycleEffect>,
    shutdown_steps: ShutdownSteps,
    read_only: bool,
}

#[derive(Default)]
struct ShutdownSteps {
    worker: bool,
    settings: bool,
    marker: bool,
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
            view.palette_shortcut = settings.value().command_palette_shortcut.clone();
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
            terminal_sequence: 0,
            terminal_capacity: 64,
            settings,
            accepting_commands: true,
            lifecycle: Lifecycle::default(),
            lifecycle_effects: Vec::new(),
            shutdown_steps: ShutdownSteps::default(),
            read_only: false,
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
    pub fn presentation(
        &self,
    ) -> (
        crate::app::settings::Theme,
        crate::app::settings::DisplayDensity,
    ) {
        self.settings
            .as_ref()
            .map_or((Default::default(), Default::default()), |settings| {
                (settings.value().theme, settings.value().display_density)
            })
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
        // The old worker was joined above, so none of its mutations remain unknown. Keep records
        // for correlation diagnostics; generation checks make late responses harmless.
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
        // Retain terminal/in-flight records for diagnostics. The worker was joined first.
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
                    command_id: Some(cid),
                    correlation_id: Some(corr),
                    generation,
                    result,
                    user_error,
                    ..
                } => self.handle_command_response(id, cid, corr, generation, result, user_error),
                StorageResponse::Completed { id, generation, .. }
                    if generation == self.generation =>
                {
                    self.view.complete_request(id);
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
        if action == ApplicationAction::RequestExit {
            self.request_exit();
            return;
        }
        if let ApplicationAction::Ui(intent) = action {
            // Global intentions are handled by the application router and are not
            // mistaken for persistence work.
            if intent == crate::app::command::AppCommand::ToggleInspector {
                self.view.inspector_visible = !self.view.inspector_visible;
            }
            return;
        }
        if self.generation.budget == 0 {
            return;
        }
        let key = DeduplicationKey(format!("{:?}", action));
        if self
            .pending_commands
            .values()
            .any(|c| !c.status.is_terminal() && c.deduplication_key == key)
        {
            return;
        }
        let id = self.next_command;
        self.next_command = self.next_command.saturating_add(1);
        let envelope = CommandEnvelope {
            command_id: id,
            correlation_id: id,
            budget_generation: self.generation.budget,
            payload: action,
        };
        let ApplicationAction::Financial(financial) = &envelope.payload else {
            return;
        };
        let confirmation = if requires_confirmation(financial) {
            ConfirmationState::Required
        } else {
            ConfirmationState::NotRequired
        };
        let mut command = RuntimeCommand {
            envelope,
            status: CommandStatus::Queued,
            worker_request_id: None,
            confirmation,
            focus_restoration_id: Some(id),
            operation_label: "Financial operation".into(),
            reversibility: Reversibility::Reversible,
            cancellation_policy: CancellationPolicy::Cancellable,
            retry: RetryMetadata {
                attempts: 0,
                max_attempts: 3,
            },
            operation_class: OperationClass::Mutation,
            safe_failure: None,
            deduplication_key: key,
            terminal_sequence: None,
        };
        if matches!(confirmation, ConfirmationState::Required) {
            command
                .transition(CommandStatus::AwaitingConfirmation)
                .expect("legal transition");
            self.view.notifications.push(Notification {
                kind: NotificationKind::Warning,
                title: "Confirmation required".into(),
                detail: "Review and confirm this change before continuing.".into(),
                persistent: true,
            });
            self.pending_commands.insert(id, command);
            return;
        }
        self.pending_commands.insert(id, command);
        self.submit_runtime_command(id);
    }
    pub fn confirm_command(&mut self, id: u64, token: u64) -> bool {
        let Some(c) = self.pending_commands.get_mut(&id) else {
            return false;
        };
        if c.status != CommandStatus::AwaitingConfirmation {
            return false;
        }
        c.confirmation = ConfirmationState::Confirmed(token);
        self.submit_runtime_command(id);
        true
    }
    fn submit_runtime_command(&mut self, id: u64) {
        let request = self.allocate_request();
        let Some(c) = self.pending_commands.get_mut(&id) else {
            return;
        };
        if c.envelope.budget_generation != self.generation.budget {
            return Self::fail_record(
                c,
                FailureSafety::NonRetryable(crate::storage::worker::SafeUserError {
                    message: "This operation belongs to a closed budget.",
                }),
                &mut self.terminal_sequence,
            );
        }
        if c.transition(CommandStatus::Submitting).is_err() {
            return;
        }
        c.worker_request_id = Some(request);
        c.retry.attempts += 1;
        let req = crate::storage::worker::StorageRequest {
            id: request,
            generation: self.generation,
            operation: crate::storage::worker::WorkerOperation::Financial(
                crate::storage::worker::FinancialOperation::Command(c.envelope.clone()),
            ),
        };
        if self.worker.as_ref().is_some_and(|w| w.submit(req).is_ok()) {
            c.transition(CommandStatus::Running)
                .expect("submitting may run");
        } else {
            Self::fail_record(
                c,
                FailureSafety::Retryable(crate::storage::worker::SafeUserError {
                    message: "The operation could not be submitted. You may safely retry.",
                }),
                &mut self.terminal_sequence,
            )
        }
    }
    fn fail_record(c: &mut RuntimeCommand, f: FailureSafety, seq: &mut u64) {
        let _ = c.transition(CommandStatus::Failed);
        c.safe_failure = Some(f);
        *seq += 1;
        c.terminal_sequence = Some(*seq);
    }
    fn handle_command_response(
        &mut self,
        id: u64,
        cid: u64,
        corr: u64,
        generation: Generation,
        result: Result<crate::storage::worker::TypedResult, crate::storage::worker::WorkerError>,
        safe: Option<crate::storage::worker::SafeUserError>,
    ) {
        let Some(c) = self.pending_commands.get_mut(&cid) else {
            return;
        };
        if generation != self.generation
            || !c.response_matches(id, cid, corr, generation.budget)
            || c.status != CommandStatus::Running
        {
            return;
        }
        self.view.complete_request(id);
        match result { Ok(crate::storage::worker::TypedResult::Mutation(m))=>{c.transition(CommandStatus::Committed).expect("running commits");self.terminal_sequence+=1;c.terminal_sequence=Some(self.terminal_sequence);if let (ApplicationAction::Financial(command),Some(crate::storage::protocol::UndoData::Command(inverse)))=(&c.envelope.payload,m.undo){self.history.record_success(HistoryEntry{label:c.operation_label.clone(),command:command.clone(),inverse});}self.invalidations.merge(m.invalidations);}, Err(crate::storage::worker::WorkerError::Cancelled)=>{let _=c.transition(CommandStatus::Cancelled);self.terminal_sequence+=1;c.terminal_sequence=Some(self.terminal_sequence);}, Err(_)=>Self::fail_record(c,FailureSafety::Retryable(safe.unwrap_or(crate::storage::worker::SafeUserError{message:"The operation failed without changing your data. You may safely retry."})),&mut self.terminal_sequence), _=>Self::fail_record(c,FailureSafety::NonRetryable(crate::storage::worker::SafeUserError{message:"The worker returned an unexpected result."}),&mut self.terminal_sequence)}
        self.prune_commands();
    }
    pub fn cancel_command(&mut self, id: u64) -> bool {
        let Some(c) = self.pending_commands.get_mut(&id) else {
            return false;
        };
        if c.cancellation_policy != CancellationPolicy::Cancellable
            || !matches!(
                c.status,
                CommandStatus::Queued | CommandStatus::AwaitingConfirmation
            )
        {
            return false;
        }
        c.transition(CommandStatus::Cancelled).is_ok()
    }
    fn prune_commands(&mut self) {
        let mut terminal = self
            .pending_commands
            .iter()
            .filter(|(_, c)| c.status.is_terminal())
            .map(|(id, c)| (*id, c.terminal_sequence.unwrap_or(0)))
            .collect::<Vec<_>>();
        terminal.sort_by_key(|x| x.1);
        let remove = terminal.len().saturating_sub(self.terminal_capacity);
        for (id, _) in terminal.into_iter().take(remove) {
            self.pending_commands.remove(&id);
        }
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
    pub const fn lifecycle_state(&self) -> LifecycleState {
        self.lifecycle.state()
    }
    pub const fn is_read_only(&self) -> bool {
        self.read_only
    }
    pub fn take_lifecycle_effects(&mut self) -> Vec<LifecycleEffect> {
        std::mem::take(&mut self.lifecycle_effects)
    }
    pub fn native_close_requested(&mut self) {
        let effects = self.lifecycle.native_close_requested();
        self.lifecycle_effects.extend(effects);
        self.advance_exit_review();
    }
    pub fn request_exit(&mut self) {
        let effects = self.lifecycle.request_exit();
        self.lifecycle_effects.extend(effects);
        self.advance_exit_review();
    }
    fn advance_exit_review(&mut self) {
        if self.lifecycle.state() == LifecycleState::ShutdownRequested {
            self.lifecycle
                .begin_review()
                .expect("requested shutdown can be reviewed");
            let effect = self
                .lifecycle
                .begin_shutdown()
                .expect("review can begin shutdown");
            self.lifecycle_effects.push(effect);
        }
    }

    /// The sole ordered shutdown state machine. A marker is published only after every
    /// required durability step has succeeded.
    pub fn shutdown(&mut self) -> Result<(), String> {
        if self.shutdown_steps.marker {
            return Ok(());
        }
        if self.lifecycle.state() == LifecycleState::Running {
            self.request_exit();
        }
        if self.lifecycle.state() == LifecycleState::ShutdownFailed {
            let effect = self
                .lifecycle
                .retry_shutdown()
                .map_err(|e| format!("illegal retry: {e:?}"))?;
            self.lifecycle_effects.push(effect);
        }
        self.accepting_commands = false;
        for command in self.pending_commands.values_mut().filter(|c| {
            matches!(
                c.status,
                CommandStatus::Queued | CommandStatus::AwaitingConfirmation
            )
        }) {
            if command.cancellation_policy == CancellationPolicy::Cancellable {
                let _ = command.transition(CommandStatus::Cancelled);
            }
        }
        if !self.view.operations.is_empty() {
            self.view.operations.clear();
        }
        if !self.shutdown_steps.worker {
            if let Some(mut worker) = self.worker.take() {
                if let Err(error) = worker.shutdown() {
                    self.read_only = true;
                    return self.fail_shutdown("worker stop/join and WAL checkpoint", error);
                }
            }
            self.shutdown_steps.worker = true;
        }
        if !self.shutdown_steps.settings {
            if let Err(error) = self.save_settings() {
                return self.fail_shutdown("settings persistence", error);
            }
            self.shutdown_steps.settings = true;
        }
        let paths = self
            .paths
            .as_ref()
            .ok_or_else(|| Self::shutdown_failed("clean marker", "portable paths unavailable"))?;
        if !self.shutdown_steps.marker {
            if let Err(error) = write_clean_marker(&paths.data.join(".clean-shutdown")) {
                return self.fail_shutdown("clean marker write/sync", error);
            }
            self.shutdown_steps.marker = true;
        }
        let effects = self
            .lifecycle
            .shutdown_succeeded()
            .map_err(|e| format!("illegal shutdown completion: {e:?}"))?;
        self.lifecycle_effects.extend(effects);
        Ok(())
    }
    fn fail_shutdown(&mut self, step: &str, error: impl std::fmt::Display) -> Result<(), String> {
        let message = Self::shutdown_failed(step, error);
        if self.lifecycle.state() == LifecycleState::ShuttingDown {
            if let Ok(effect) = self.lifecycle.shutdown_failed() {
                self.lifecycle_effects.push(effect);
            }
        }
        Err(message)
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
    use crate::app::command::{InboxAction, InboxCommand};

    fn inbox_action() -> ApplicationAction {
        ApplicationAction::Financial(FinancialCommand::Inbox(InboxCommand::Resolve {
            item_id: crate::app::inbox::InboxItemId::FailedOperation("stable-id".into()),
            action: InboxAction::Dismiss,
        }))
    }

    fn runtime_with_worker() -> (tempfile::TempDir, ApplicationRuntime) {
        let dir = tempfile::tempdir().unwrap();
        let database = dir.path().join("runtime.sqlite3");
        let worker = StorageWorker::start(&database, || {}).unwrap();
        let mut runtime = ApplicationRuntime::new(
            None,
            None,
            false,
            StartupContext {
                marker_was_absent: false,
                last_successfully_opened_budget: None,
            },
        );
        runtime.commit_session(
            BudgetSession {
                budget_id: crate::domain::BudgetId::new(),
                database_path: database,
                schema_version: 3,
                summary: crate::app::session::SessionSummary {
                    budget_name: "Test".into(),
                    account_count: 0,
                },
            },
            worker,
        );
        (dir, runtime)
    }

    #[test]
    fn repeated_semantic_action_submits_once_and_commits_history_once() {
        let (_dir, mut runtime) = runtime_with_worker();
        runtime.dispatch(inbox_action());
        runtime.dispatch(inbox_action()); // click/key repeat while equivalent work is active
        assert_eq!(runtime.pending_commands.len(), 1);
        assert_eq!(
            runtime.pending_commands.values().next().unwrap().status,
            CommandStatus::Running
        );
        let response = runtime
            .worker
            .as_ref()
            .unwrap()
            .response_timeout(std::time::Duration::from_secs(2))
            .unwrap();
        let StorageResponse::Completed {
            id,
            command_id: Some(command_id),
            correlation_id: Some(correlation_id),
            generation,
            result,
            user_error,
            ..
        } = response
        else {
            panic!("expected associated completion")
        };
        runtime.handle_command_response(
            id,
            command_id,
            correlation_id,
            generation,
            result,
            user_error,
        );
        assert_eq!(runtime.history.undo_len(), 1);
        assert_eq!(
            runtime.pending_commands[&command_id].status,
            CommandStatus::Committed
        );
    }

    #[test]
    fn stale_budget_response_cannot_commit_or_clear_redo() {
        let (_dir, mut runtime) = runtime_with_worker();
        runtime.history.record_success(HistoryEntry {
            label: "old".into(),
            command: match inbox_action() {
                ApplicationAction::Financial(c) => c,
                _ => unreachable!(),
            },
            inverse: match inbox_action() {
                ApplicationAction::Financial(c) => c,
                _ => unreachable!(),
            },
        });
        assert!(runtime.history.undo().is_some());
        let redo_before = runtime.history.redo_len();
        runtime.dispatch(inbox_action());
        let c = runtime.pending_commands.values().next().unwrap().clone();
        runtime.generation.budget += 1;
        runtime.handle_command_response(
            c.worker_request_id.unwrap(),
            c.envelope.command_id,
            c.envelope.correlation_id,
            Generation {
                budget: c.envelope.budget_generation,
                view: 0,
            },
            Ok(crate::storage::worker::TypedResult::Mutation(
                crate::storage::protocol::MutationResult {
                    command_id: c.envelope.command_id,
                    correlation_id: c.envelope.correlation_id,
                    operation_label: "test",
                    affected_entity_ids: vec![],
                    undo: None,
                    invalidations: Default::default(),
                    navigation: None,
                    focus_restoration: None,
                    notice: None,
                },
            )),
            None,
        );
        assert_eq!(runtime.history.undo_len(), 0);
        assert_eq!(runtime.history.redo_len(), redo_before);
    }
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
            command_id: None,
            correlation_id: None,
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
        command.dispatch(ApplicationAction::RequestExit);
        assert_eq!(command.lifecycle_state(), LifecycleState::ShuttingDown);
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
        window.native_close_requested();
        assert_eq!(window.lifecycle_state(), LifecycleState::ShuttingDown);
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
