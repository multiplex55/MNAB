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
}

impl ApplicationRuntime {
    pub fn new(
        paths: Option<PortablePaths>,
        settings: Option<SettingsSession>,
        malformed_settings: bool,
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
        Self {
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
        }
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
        if let Some(mut old) = self.worker.take() {
            let _ = old.shutdown();
        }
        self.session = Some(session);
        self.worker = Some(worker);
        self.generation.budget = self
            .generation
            .budget
            .checked_add(1)
            .expect("budget generation exhausted");
        self.generation.view = 0;
        self.view.generation = self.generation;
    }
    pub fn close_session(&mut self) {
        if let Some(mut worker) = self.worker.take() {
            let _ = worker.shutdown();
        }
        self.session = None;
        self.generation.budget = self.generation.budget.saturating_add(1);
        self.generation.view = 0;
        self.view.generation = self.generation;
    }
    pub fn drain_worker_responses(&mut self) {
        while let Some(response) = self.worker.as_ref().and_then(StorageWorker::try_response) {
            match response {
                StorageResponse::Completed {
                    id,
                    generation,
                    result,
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
            }
        }
    }
    pub fn dispatch_collected(&mut self, actions: ActionCollector) {
        for action in actions.into_actions() {
            self.dispatch(action);
        }
    }
    fn dispatch(&mut self, action: ApplicationAction) {
        if let ApplicationAction::Ui(intent) = action {
            // Global intentions are handled by the application router and are not
            // mistaken for persistence work.
            if intent == crate::app::command::AppCommand::ToggleInspector {
                self.view.inspector_visible = !self.view.inspector_visible;
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
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn request_ids_are_monotonic_and_generations_are_independent() {
        let mut runtime = ApplicationRuntime::new(None, None, false);
        assert_eq!(
            (runtime.allocate_request(), runtime.allocate_request()),
            (1, 2)
        );
        let before = runtime.generation();
        runtime.bump_view_generation();
        assert_eq!(runtime.generation().budget, before.budget);
        assert_eq!(runtime.generation().view, before.view + 1);
    }
}
