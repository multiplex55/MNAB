use std::collections::BTreeMap;

use crate::{
    app::{
        command::{
            AppCommand, ApplicationAction, CancellationPolicy, CategoryAction, CommandEnvelope,
            CommandHistory, CommandId, CommandStatus, ConfirmationState, DeduplicationKey,
            FailureSafety, FinancialCommand, HistoryEntry, OperationClass, RetryMetadata,
            Reversibility, RuntimeCommand, TransactionCommand,
        },
        dispatcher::{ActionCollector, requires_confirmation},
        lifecycle::{DatabaseLifecycle, Lifecycle, LifecycleEffect, LifecycleState},
        portable_paths::PortablePaths,
        session::BudgetSession,
        settings::SettingsSession,
        startup::StartupContext,
        state::{AppState, Notification, NotificationKind},
        view_invalidation::ViewInvalidations,
    },
    domain::AccountId,
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
    history_operations: BTreeMap<CommandId, bool>,
    invalidations: ViewInvalidations,
    pending_commands: BTreeMap<u64, RuntimeCommand>,
    terminal_sequence: u64,
    terminal_capacity: usize,
    settings: Option<SettingsSession>,
    accepting_commands: bool,
    lifecycle: Lifecycle,
    database_lifecycle: DatabaseLifecycle,
    lifecycle_effects: Vec<LifecycleEffect>,
    shutdown_steps: ShutdownSteps,
    read_only: bool,
    hydrated_generation: Option<u64>,
}

#[derive(Default)]
struct ShutdownSteps {
    worker: bool,
    settings: bool,
    marker: bool,
}

impl ApplicationRuntime {
    pub fn rename_budget(
        &mut self,
        catalog: &mut crate::app::budget_catalog::BudgetCatalog,
        id: crate::domain::BudgetId,
        name: &str,
    ) -> Result<(), crate::app::budget_catalog::CatalogError> {
        let paths = self
            .paths
            .as_ref()
            .ok_or(crate::app::budget_catalog::CatalogError::UnmanagedPath)?;
        catalog.rename(paths, id, name)?;
        if let Some(session) = self.session.as_mut().filter(|s| s.budget_id == id) {
            session.summary.budget_name = name.trim().to_owned();
            self.view
                .budget_name
                .clone_from(&session.summary.budget_name);
        }
        Ok(())
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
            history_operations: BTreeMap::new(),
            invalidations: ViewInvalidations::default(),
            pending_commands: BTreeMap::new(),
            terminal_sequence: 0,
            terminal_capacity: 64,
            settings,
            accepting_commands: true,
            lifecycle: Lifecycle::default(),
            database_lifecycle: DatabaseLifecycle::Initializing,
            lifecycle_effects: Vec::new(),
            shutdown_steps: ShutdownSteps::default(),
            read_only: false,
            hydrated_generation: None,
        };
        runtime.apply_startup(startup);
        runtime
    }

    fn apply_startup(&mut self, startup: StartupContext) {
        let Some(paths) = self.paths.clone() else {
            self.database_lifecycle = DatabaseLifecycle::RecoveryRequired;
            return;
        };
        if !startup.fixed_database_exists {
            self.database_lifecycle = DatabaseLifecycle::FirstRunRequired;
            self.view.open_dialog(
                crate::app::state::Dialog::Onboarding,
                egui::Id::new("startup"),
                egui::Id::new("toolbar"),
            );
            return;
        }
        self.database_lifecycle = DatabaseLifecycle::OpeningDatabase;
        let result = crate::app::budget_catalog::BudgetCatalog::default().prepare_fixed_checked(
            &paths,
            startup.marker_was_absent,
            || {},
        );
        match result {
            Ok(prepared) => {
                self.commit_session(prepared.session, prepared.worker);
                self.database_lifecycle = DatabaseLifecycle::Ready;
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
                self.database_lifecycle = DatabaseLifecycle::RecoveryRequired;
                self.view.open_dialog(
                    crate::app::state::Dialog::RecoveryChoice,
                    egui::Id::new("startup"),
                    egui::Id::new("toolbar"),
                );
            }
        }
    }
    fn resolve_startup_destination(
        &mut self,
        tree: &[crate::storage::query_store::AccountTreeGroup],
    ) {
        let accounts: Vec<_> = tree
            .iter()
            .flat_map(|g| &g.accounts)
            .filter_map(|a| {
                a.id.parse()
                    .ok()
                    .map(|id| crate::app::startup::StartupAccount {
                        id,
                        favorite: a.favorite,
                        closed: a.closed,
                    })
            })
            .collect();
        let groups: Vec<_> = tree
            .iter()
            .filter_map(|g| g.id.as_deref()?.parse().ok())
            .collect();
        let last = self
            .settings
            .as_ref()
            .and_then(|s| s.value().last_selected_account_id.as_deref());
        let destination = crate::app::startup::resolve_destination(last, &accounts);
        if let Some(settings) = &mut self.settings {
            settings
                .value_mut()
                .repair_persisted_ids(&accounts.iter().map(|a| a.id).collect::<Vec<_>>(), &groups);
        }
        match destination {
            crate::app::startup::StartupDestination::Workspace(
                crate::app::navigation::Workspace::Account(id),
            ) => self.select_account(id),
            crate::app::startup::StartupDestination::Workspace(workspace) => {
                self.view.navigation.workspace = workspace
            }
            crate::app::startup::StartupDestination::AccountOnboarding => {
                self.view.editor = crate::app::state::EditorState::CreatingAccount(
                    crate::app::state::AccountEditorState {
                        account_id: None,
                        form: Default::default(),
                        metadata: crate::app::state::EditorMetadata::new(egui::Id::new(
                            "account-onboarding",
                        )),
                    },
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
            settings.value_mut().last_selected_account_id =
                self.view.selected_account.map(|id| id.to_string());
            settings.value_mut().last_workspace = Some(
                match self.view.navigation.workspace {
                    crate::app::navigation::Workspace::Overview => "overview",
                    crate::app::navigation::Workspace::Budget => "budget",
                    crate::app::navigation::Workspace::Account(_) => "account",
                    crate::app::navigation::Workspace::AllTransactions => "all_transactions",
                    crate::app::navigation::Workspace::Categories => "categories",
                    crate::app::navigation::Workspace::Reports => "reports",
                    crate::app::navigation::Workspace::Inbox => "inbox",
                }
                .into(),
            );
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
    pub const fn database_lifecycle(&self) -> DatabaseLifecycle {
        self.database_lifecycle
    }
    pub fn view_mut(&mut self) -> &mut AppState {
        self.view.can_undo = self.history.can_undo();
        self.view.can_redo = self.history.can_redo();
        self.view.undo_label = self.history.undo_label().map(str::to_owned);
        self.view.redo_label = self.history.redo_label().map(str::to_owned);
        &mut self.view
    }
    /// Immutable application snapshot for black-box journeys and non-egui hosts.
    #[must_use]
    pub const fn view(&self) -> &AppState {
        &self.view
    }

    /// Whether the active, committed session still owns its storage worker.
    #[must_use]
    pub const fn worker_available(&self) -> bool {
        self.worker.is_some()
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
    /// Account changes invalidate account-dependent projections and update the default inspector.
    pub fn select_account(&mut self, account: AccountId) {
        if self.view.selected_account == Some(account) {
            return;
        }
        self.view.selected_account = Some(account);
        self.view.navigation.workspace = crate::app::navigation::Workspace::Account(account);
        self.view.inspector_context =
            crate::app::state::InspectorContext::AccountSummary(Some(account));
        self.bump_view_generation();
        self.invalidations
            .insert(crate::app::view_invalidation::ViewInvalidation::AccountRegister(account));
    }

    fn cancel_editor(&mut self) {
        if let Some(focus) = self.view.editor.metadata().map(|m| m.restore_focus) {
            self.view.register_focus = Some(focus);
        }
        self.view.editor = crate::app::state::EditorState::Idle;
    }

    /// Validate the complete workflow model and turn it into a domain command.
    /// The editor deliberately remains mounted until the correlated worker
    /// response succeeds, so validation and storage errors never erase input.
    fn commit_editor(&mut self) {
        use crate::app::state::EditorState;
        if self
            .view
            .editor
            .metadata_mut()
            .is_some_and(|metadata| !metadata.begin_validation())
        {
            return;
        }
        let Some(budget_id) = self.view.active_budget else {
            self.fail_editor("Open a budget before saving.");
            return;
        };
        if let EditorState::CreatingTransaction(editor) | EditorState::EditingTransaction(editor) =
            &mut self.view.editor
        {
            match editor.build_transaction(budget_id) {
                Ok(_) => editor.errors = Default::default(),
                Err(errors) => {
                    editor.errors = errors;
                    editor.metadata.commit_state = crate::app::state::CommitState::Failed;
                    editor.metadata.pending_command_id = None;
                    // Transaction validation is intentionally not copied into the generic
                    // string bag: the register renders the structured field errors.
                    editor.metadata.validation_errors.clear();
                    return;
                }
            }
        }
        let command = match &self.view.editor {
            EditorState::CreatingAccount(editor) | EditorState::EditingAccount(editor) => editor
                .form
                .validate()
                .map(|(name, account_type, opening_magnitude, opening_date)| {
                    let mut account = crate::domain::Account::new(budget_id, name, account_type);
                    account.group_id = editor.form.group_id;
                    account.note = (!editor.form.note.trim().is_empty())
                        .then(|| editor.form.note.trim().to_owned());
                    account.favorite = editor.form.favorite;
                    if let Some(id) = editor.account_id {
                        account.id = id;
                    }
                    FinancialCommand::Account(if editor.account_id.is_some() {
                        crate::app::command::AccountCommand::Update(account)
                    } else {
                        crate::app::command::AccountCommand::CreateWithOpening {
                            account,
                            opening_magnitude,
                            opening_date,
                        }
                    })
                })
                .map_err(str::to_owned),
            EditorState::CreatingTransaction(editor) | EditorState::EditingTransaction(editor) => {
                editor
                    .build_transaction(budget_id)
                    .map(|transaction| {
                        FinancialCommand::Transaction(TransactionCommand::Save(transaction))
                    })
                    .map_err(|errors| {
                        errors
                            .form
                            .or(errors.amount)
                            .or(errors.date)
                            .or(errors.account)
                            .unwrap_or_else(|| "Transaction has invalid fields".into())
                    })
            }
            EditorState::CreatingTransfer(editor) => editor
                .draft
                .validate()
                .map_err(|e| format!("{e:?}"))
                .and_then(|(from, to, date, amount)| {
                    let summary_account = |id| {
                        self.view
                            .accounts
                            .iter()
                            .find(|account| account.id == id)
                            .ok_or_else(|| "Choose an available account.".to_owned())
                    };
                    let source_summary = summary_account(from)?;
                    let destination_summary = summary_account(to)?;
                    let mut source_account = crate::domain::Account::new(
                        budget_id,
                        &source_summary.name,
                        source_summary.account_type,
                    );
                    source_account.id = from;
                    let mut destination_account = crate::domain::Account::new(
                        budget_id,
                        &destination_summary.name,
                        destination_summary.account_type,
                    );
                    destination_account.id = to;
                    let transfer_id = crate::domain::TransferId::new();
                    let source_amount = amount.checked_neg().map_err(|error| error.to_string())?;
                    let (source_body, destination_body) =
                        crate::domain::TransactionBody::categorized_transfer(
                            transfer_id,
                            &source_account,
                            source_amount,
                            &destination_account,
                            amount,
                            editor.draft.category_id,
                            editor.draft.category_effect_account,
                        )
                        .map_err(|error| error.to_string())?;
                    let make = |account_id, leg_amount, body| crate::domain::Transaction {
                        id: crate::domain::TransactionId::new(),
                        budget_id,
                        account_id,
                        date,
                        payee_id: None,
                        amount: leg_amount,
                        memo: (!editor.draft.memo.trim().is_empty())
                            .then(|| editor.draft.memo.trim().to_owned()),
                        clearance: crate::domain::Clearance::Uncleared,
                        approval: crate::domain::Approval::Approved,
                        body,
                        archived: false,
                        voided: false,
                    };
                    Ok(FinancialCommand::Transaction(
                        TransactionCommand::SaveTransfer {
                            source: make(from, source_amount, source_body),
                            destination: make(to, amount, destination_body),
                        },
                    ))
                }),
            EditorState::Reconciling(editor) => (|| {
                let account_id = editor
                    .account_id
                    .ok_or_else(|| "Choose an account.".to_owned())?;
                let statement_date = time::Date::parse(
                    editor.statement_date.trim(),
                    &time::format_description::well_known::Iso8601::DATE,
                )
                .map_err(|_| "Statement date must be YYYY-MM-DD.".to_owned())?;
                let ending_balance = editor
                    .statement_balance
                    .parse::<crate::domain::Money>()
                    .map_err(|_| "Enter a valid statement balance.".to_owned())?;
                let cleared = self
                    .view
                    .accounts
                    .iter()
                    .find(|account| account.id == account_id)
                    .map_or(crate::domain::Money::ZERO, |account| {
                        account.cleared_balance
                    });
                let difference = crate::domain::reconciliation_difference(ending_balance, cleared)
                    .map_err(|error| error.to_string())?;
                if difference != crate::domain::Money::ZERO {
                    return Err(
                        "Reconciliation can complete only when the difference is exactly zero."
                            .into(),
                    );
                }
                Ok(FinancialCommand::Reconciliation(
                    crate::app::command::ReconciliationCommand::CompleteSnapshot(
                        crate::domain::Reconciliation {
                            id: crate::domain::ReconciliationId::new(),
                            budget_id,
                            account_id,
                            statement_date: crate::domain::StatementDate(statement_date),
                            ending_balance,
                            calculated_cleared_balance: cleared,
                            difference,
                            included_transaction_ids: vec![],
                            state: crate::domain::ReconciliationState::Completed,
                            created_at: time::OffsetDateTime::now_utc(),
                            completed_at: Some(time::OffsetDateTime::now_utc()),
                            invalidated_at: None,
                        },
                    ),
                ))
            })(),
            _ => Err("Complete the required workflow details before saving.".to_owned()),
        };
        let command = match command {
            Ok(command) => command,
            Err(error) => {
                self.fail_editor(error);
                return;
            }
        };
        let command_id = self.next_command;
        if let Some(metadata) = self.view.editor.metadata_mut() {
            metadata.begin_submission(command_id);
        }
        self.dispatch(ApplicationAction::Financial(command));
        if self
            .pending_commands
            .get(&command_id)
            .is_none_or(|c| c.status == CommandStatus::Failed)
        {
            let message = self
                .pending_commands
                .get(&command_id)
                .and_then(|c| c.safe_failure.as_ref())
                .map_or_else(
                    || "The operation could not be submitted. You may retry.".to_owned(),
                    |e| format!("{e:?}"),
                );
            self.fail_editor(message);
        }
    }

    fn fail_editor(&mut self, error: impl Into<String>) {
        if let Some(metadata) = self.view.editor.metadata_mut() {
            metadata.fail(error);
        }
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
        let now =
            time::OffsetDateTime::now_local().unwrap_or_else(|_| time::OffsetDateTime::now_utc());
        self.view.selected_month =
            crate::domain::BudgetMonth::new(now.year(), u8::from(now.month()))
                .expect("current calendar month is valid");
        self.view.database_path = Some(database_path);
        self.view.budget_name = budget_name;
        self.hydrated_generation = None;
        self.hydrate_session();
    }
    /// Schedules the immutable projections needed to render a newly opened session.
    /// Repeated calls for the same budget generation are deliberately idempotent.
    pub fn hydrate_session(&mut self) {
        let Some(budget_id) = self.view.active_budget else {
            return;
        };
        if self.hydrated_generation == Some(self.generation.budget) {
            return;
        }
        self.hydrated_generation = Some(self.generation.budget);

        let account_id = self.allocate_request();
        self.view
            .account_tree
            .begin(account_id, self.generation, None);
        let month_id = self.allocate_request();
        self.view
            .budget_month
            .begin(month_id, self.generation, None);
        let inbox_id = self.allocate_request();
        self.view
            .inbox_summary
            .begin(inbox_id, self.generation, None);
        self.request_category_catalog(None);

        let requests = [
            crate::storage::worker::StorageRequest {
                id: account_id,
                generation: self.generation,
                operation: crate::storage::worker::WorkerOperation::Account(
                    crate::storage::worker::AccountViewOperation::Tree { budget_id },
                ),
            },
            crate::storage::worker::StorageRequest {
                id: month_id,
                generation: self.generation,
                operation: crate::storage::worker::WorkerOperation::Budget(
                    crate::storage::worker::BudgetViewOperation::Month {
                        budget_id,
                        month: self.view.selected_month,
                    },
                ),
            },
            crate::storage::worker::StorageRequest {
                id: inbox_id,
                generation: self.generation,
                operation: crate::storage::worker::WorkerOperation::Inbox(
                    crate::storage::worker::InboxViewOperation::Summary { budget_id },
                ),
            },
        ];
        for request in requests {
            if self
                .worker
                .as_ref()
                .is_none_or(|worker| worker.submit(request).is_err())
            {
                let message = "Session projection could not be requested.";
                let _ = self
                    .view
                    .account_tree
                    .fail(account_id, self.generation, message);
                let _ = self
                    .view
                    .budget_month
                    .fail(month_id, self.generation, message);
                let _ = self
                    .view
                    .inbox_summary
                    .fail(inbox_id, self.generation, message);
                break;
            }
        }
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
        self.hydrated_generation = None;
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
                StorageResponse::Completed {
                    id,
                    generation,
                    result,
                    user_error,
                    ..
                } if generation == self.generation => {
                    self.handle_view_response(id, generation, result, user_error)
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
        // Mutations are deliberately collected for the whole drain.  Scheduling here, rather
        // than in `handle_command_response`, means two commands completing in the same frame
        // cannot issue duplicate reads for the same projection.
        self.process_invalidations();
    }

    /// Coalesce successful-mutation invalidations and refresh only projections which are
    /// currently materialized. Each request goes through the normal correlated `begin` path,
    /// retaining request/generation identity so an older refresh cannot win a race.
    pub fn process_invalidations(&mut self) {
        use crate::app::view_model::RegisterScope;
        use crate::app::{navigation::Workspace, view_invalidation::ViewInvalidation as V};

        let pending = std::mem::take(&mut self.invalidations);
        if pending.is_empty() || self.view.active_budget.is_none() {
            return;
        }

        let mut account_tree = false;
        let mut budget_month = false;
        let mut inbox = false;
        let mut register = false;
        let mut report = false;
        let workspace = self.view.navigation.workspace;
        for invalidation in pending.iter() {
            match invalidation {
                V::Accounts => account_tree = true,
                V::BudgetMonth(month) => budget_month |= *month == self.view.selected_month,
                V::BudgetRolloverFrom(month) => {
                    self.view.budget_month_cache.invalidate_from(*month);
                    budget_month |= self.view.selected_month >= *month;
                }
                V::Inbox => inbox = true,
                V::Reports => report = true,
                V::AllAccountRegisters => {
                    register |= matches!(workspace, Workspace::Account(_));
                }
                V::AccountRegister(account) => {
                    register |=
                        matches!(workspace, Workspace::Account(active) if active == *account);
                }
                V::AllTransactions => register |= workspace == Workspace::AllTransactions,
                _ => {}
            }
        }

        if account_tree {
            self.request_account_tree();
        }
        if budget_month {
            self.request_budget_month(self.view.selected_month);
        }
        if inbox {
            self.request_inbox_summary();
        }
        if register {
            let expected_scope = match workspace {
                Workspace::Account(id) => Some(RegisterScope::Account(id)),
                Workspace::AllTransactions => Some(RegisterScope::AllTransactions),
                _ => None,
            };
            if self
                .view
                .register_query
                .active_request
                .as_ref()
                .is_some_and(|request| Some(request.scope) == expected_scope)
            {
                self.request_active_register();
            }
        }
        if report && workspace == Workspace::Reports {
            if let Some(request) = self.view.report_query.current_request.clone() {
                self.request_report(request);
            }
        }
    }

    fn submit_view_request(&self, request: crate::storage::worker::StorageRequest) -> bool {
        self.worker
            .as_ref()
            .is_some_and(|worker| worker.submit(request).is_ok())
    }

    fn request_account_tree(&mut self) {
        let Some(budget_id) = self.view.active_budget else {
            return;
        };
        let id = self.allocate_request();
        self.view.account_tree.begin(id, self.generation, None);
        let request = crate::storage::worker::StorageRequest {
            id,
            generation: self.generation,
            operation: crate::storage::worker::WorkerOperation::Account(
                crate::storage::worker::AccountViewOperation::Tree { budget_id },
            ),
        };
        if !self.submit_view_request(request) {
            let _ = self.view.account_tree.fail(
                id,
                self.generation,
                "Account balances could not be refreshed.",
            );
        }
    }

    fn request_budget_month(&mut self, month: crate::domain::BudgetMonth) {
        let Some(budget_id) = self.view.active_budget else {
            return;
        };
        let id = self.allocate_request();
        self.view.budget_month.begin(id, self.generation, None);
        let request = crate::storage::worker::StorageRequest {
            id,
            generation: self.generation,
            operation: crate::storage::worker::WorkerOperation::Budget(
                crate::storage::worker::BudgetViewOperation::Month { budget_id, month },
            ),
        };
        if !self.submit_view_request(request) {
            let _ = self.view.budget_month.fail(
                id,
                self.generation,
                "Budget month could not be refreshed.",
            );
        }
    }

    fn request_inbox_summary(&mut self) {
        let Some(budget_id) = self.view.active_budget else {
            return;
        };
        let id = self.allocate_request();
        self.view.inbox_summary.begin(id, self.generation, None);
        let request = crate::storage::worker::StorageRequest {
            id,
            generation: self.generation,
            operation: crate::storage::worker::WorkerOperation::Inbox(
                crate::storage::worker::InboxViewOperation::Summary { budget_id },
            ),
        };
        if !self.submit_view_request(request) {
            let _ =
                self.view
                    .inbox_summary
                    .fail(id, self.generation, "Inbox could not be refreshed.");
        }
    }

    fn request_active_register(&mut self) {
        let Some(mut request) = self.view.register_query.active_request.clone() else {
            return;
        };
        request.cursor = None;
        let id = self.allocate_request();
        self.view
            .register_query
            .begin(id, self.generation, request.clone(), false);
        let submission = crate::storage::worker::StorageRequest {
            id,
            generation: self.generation,
            operation: crate::storage::worker::WorkerOperation::RegisterView(
                crate::storage::worker::RegisterViewOperation { request },
            ),
        };
        if !self.submit_view_request(submission) {
            let _ = self.view.register_query.fail(
                id,
                self.generation,
                "Transactions could not be refreshed.",
            );
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
            self.dispatch_ui(intent);
            return;
        }
        if let ApplicationAction::Data(intent) = action {
            self.dispatch_data(intent);
            return;
        }
        if let ApplicationAction::Category(intent) = action {
            self.dispatch_category(intent);
            return;
        }
        if let ApplicationAction::Report(intent) = action {
            self.dispatch_report(intent);
            return;
        }
        if let ApplicationAction::Register(intent) = action {
            self.dispatch_register(intent);
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

    fn dispatch_data(&mut self, intent: crate::app::command::DataAction) {
        use crate::app::{budget_catalog, command::DataAction, state::Dialog};
        let Some(paths) = self.paths.clone() else {
            return;
        };
        if self.database_lifecycle != DatabaseLifecycle::Ready {
            return;
        }
        match intent {
            risky @ (DataAction::RestoreBackup {
                confirmed: false, ..
            }
            | DataAction::Repair {
                confirmed: false, ..
            }) => {
                let dialog = if matches!(risky, DataAction::RestoreBackup { .. }) {
                    Dialog::RecoveryChoice
                } else {
                    Dialog::RepairBudget
                };
                self.view.pending_data_action = Some(risky);
                self.view.open_dialog(
                    dialog,
                    egui::Id::new("data-confirmation"),
                    egui::Id::new("data-menu"),
                );
            }
            DataAction::CreateBackup => self.report_data_result(
                "Backup created",
                budget_catalog::backup_fixed(
                    &paths,
                    crate::service::backup_service::BackupReason::Manual,
                )
                .map(|a| format!("Validated backup: {}", a.database.display()))
                .map_err(|e| e.to_string()),
            ),
            DataAction::Validate => self.report_data_result(
                "Validation results",
                budget_catalog::validate_fixed(&paths, true)
                    .map(|findings| {
                        if findings.is_empty() {
                            "Complete diagnostics passed with no findings.".into()
                        } else {
                            findings
                                .iter()
                                .map(|f| format!("{:?}: {}", f.severity, f.summary))
                                .collect::<Vec<_>>()
                                .join("\n")
                        }
                    })
                    .map_err(|e| e.to_string()),
            ),
            DataAction::RevealDataDirectory => self.report_data_result(
                "Data directory",
                budget_catalog::reveal_in_explorer(&paths.data)
                    .map(|()| paths.data.display().to_string())
                    .map_err(|e| e.to_string()),
            ),
            DataAction::RevealBackupDirectory => self.report_data_result(
                "Backup directory",
                budget_catalog::reveal_in_explorer(&paths.backups)
                    .map(|()| paths.backups.display().to_string())
                    .map_err(|e| e.to_string()),
            ),
            DataAction::RenameBudget { name } => {
                let result = self
                    .session
                    .as_ref()
                    .map(|s| s.budget_id)
                    .ok_or(budget_catalog::CatalogError::NotFound)
                    .and_then(|id| {
                        let mut catalog = budget_catalog::BudgetCatalog::default();
                        catalog.refresh(&paths)?;
                        self.rename_budget(&mut catalog, id, &name)
                    });
                self.report_data_result(
                    "Budget name updated",
                    result
                        .map(|()| format!("Budget is now named {}.", name.trim()))
                        .map_err(|e| e.to_string()),
                );
            }
            DataAction::RestoreBackup {
                metadata_path,
                confirmed: true,
            } => {
                if let Some(mut worker) = self.worker.take() {
                    let _ = worker.shutdown();
                }
                let result = budget_catalog::restore_fixed(&paths, &metadata_path)
                    .map(|a| {
                        format!(
                            "Restored and completely validated backup from {}.",
                            a.database.display()
                        )
                    })
                    .map_err(|e| e.to_string());
                self.reopen_after_data_operation(&paths);
                self.report_data_result("Restore results", result);
            }
            DataAction::Repair {
                request,
                confirmed: true,
            } => {
                if let Some(mut worker) = self.worker.take() {
                    let _ = worker.shutdown();
                }
                let result = (|| {
                    let mut catalog = budget_catalog::BudgetCatalog::default();
                    catalog.refresh(&paths)?;
                    let id = self
                        .session
                        .as_ref()
                        .ok_or(budget_catalog::CatalogError::NotFound)?
                        .budget_id;
                    catalog.repair(&paths, id, request).map(|r| {
                        format!(
                            "Repair completed and validation passed. Safety backup: {}",
                            r.backup.display()
                        )
                    })
                })()
                .map_err(|e| e.to_string());
                self.reopen_after_data_operation(&paths);
                self.report_data_result("Repair results", result);
            }
        }
        self.view.pending_data_action = None;
        self.view.dialog = None;
    }

    fn reopen_after_data_operation(&mut self, paths: &PortablePaths) {
        if let Ok(worker) = StorageWorker::start(&paths.database, || {}) {
            self.worker = Some(worker);
        }
    }

    fn report_data_result(&mut self, title: &str, result: Result<String, String>) {
        self.view.notifications.push(match result {
            Ok(detail) => Notification::success(title, detail),
            Err(detail) => Notification::actionable_error(title, detail),
        });
    }

    fn dispatch_register(&mut self, action: crate::app::command::RegisterAction) {
        use crate::app::command::RegisterAction;
        let ids = self
            .view
            .register_query
            .last_successful
            .as_ref()
            .map(|p| p.rows.iter().map(|r| r.transaction_id).collect::<Vec<_>>())
            .unwrap_or_default();
        match action {
            RegisterAction::Click { id, ctrl, shift } => {
                if shift {
                    self.view.register_selection.select_range(id, &ids);
                } else if ctrl {
                    self.view
                        .register_selection
                        .toggle(id, crate::app::register::AllMatchingClick::ToggleExclusion);
                } else {
                    self.view.register_selection.select_only(id);
                }
                self.view.selected_transaction = self.view.register_selection.cursor();
            }
            RegisterAction::Move { delta, extend } => {
                let has_more = self
                    .view
                    .register_query
                    .last_successful
                    .as_ref()
                    .is_some_and(|p| p.has_more);
                let _ = self
                    .view
                    .register_selection
                    .move_cursor(&ids, delta, extend, has_more);
                self.view.selected_transaction = self.view.register_selection.cursor();
            }
            RegisterAction::ToggleCurrent => {
                if let Some(id) = self.view.register_selection.cursor() {
                    self.view
                        .register_selection
                        .toggle(id, crate::app::register::AllMatchingClick::ToggleExclusion);
                    self.view.selected_transaction = Some(id);
                }
            }
            RegisterAction::BeginEdit(id) => {
                self.view.selected_transaction = Some(id);
                let row = self
                    .view
                    .register_query
                    .last_successful
                    .as_ref()
                    .and_then(|p| p.rows.iter().find(|r| r.transaction_id == id));
                if let Some(draft) = row.and_then(|row| {
                    crate::ui::workspaces::register::editor_from_row(
                        row,
                        crate::app::state::EditorMetadata::new(egui::Id::new("register")),
                    )
                }) {
                    self.view.editor = crate::app::state::EditorState::EditingTransaction(draft);
                } else {
                    self.view
                        .notifications
                        .push(crate::app::state::Notification {
                            kind: crate::app::state::NotificationKind::Information,
                            title: "Loading transaction details".into(),
                            detail:
                                "The complete transaction and splits are required before editing."
                                    .into(),
                            persistent: false,
                        });
                }
            }
        }
    }

    /// Exhaustive UI router: adding an `AppCommand` makes this match fail to compile until routed.
    fn dispatch_ui(&mut self, intent: AppCommand) {
        use crate::app::{
            navigation::Workspace,
            state::{
                AccountEditorState, Dialog, EditorMetadata, EditorState, GroupEditorState,
                ImportEditorState, ReconciliationEditorState, TransferEditorState,
            },
        };
        use AppCommand::*;
        if matches!(intent, Undo | Redo) {
            let redo = intent == Redo;
            let command = if redo {
                self.history.next_redo()
            } else {
                self.history.next_undo()
            };
            if let Some(command) = command {
                let id = self.next_command;
                self.dispatch(ApplicationAction::Financial(command));
                if self.pending_commands.contains_key(&id) {
                    self.history_operations.insert(id, redo);
                }
            }
            return;
        }
        let register_open = matches!(
            self.view.navigation.workspace,
            Workspace::Account(_) | Workspace::AllTransactions
        );
        if register_open && !self.view.editor.is_active() {
            match intent {
                MoveUp | MoveDown => {
                    self.dispatch_register(crate::app::command::RegisterAction::Move {
                        delta: if intent == MoveUp { -1 } else { 1 },
                        extend: false,
                    });
                    return;
                }
                ToggleSelection => {
                    self.dispatch_register(crate::app::command::RegisterAction::ToggleCurrent);
                    return;
                }
                Edit | EditTransaction | Commit => {
                    if let Some(id) = self
                        .view
                        .register_selection
                        .cursor()
                        .or(self.view.selected_transaction)
                    {
                        self.dispatch_register(crate::app::command::RegisterAction::BeginEdit(id));
                        return;
                    }
                }
                Delete | DeleteTransaction => {
                    let selected = match &self.view.register_selection {
                        crate::app::register::TransactionSelection::Explicit { ids, .. } => {
                            crate::app::command::TransactionBatchSelection::Explicit(ids.clone())
                        }
                        crate::app::register::TransactionSelection::AllMatching {
                            query,
                            exclusions,
                            ..
                        } => crate::app::command::TransactionBatchSelection::AllMatching {
                            query: query.clone(),
                            exclusions: exclusions.clone(),
                        },
                    };
                    self.dispatch(ApplicationAction::Financial(FinancialCommand::Transaction(
                        TransactionCommand::Batch(crate::app::command::TransactionBatchCommand {
                            selection: selected,
                            action: crate::app::command::TransactionBatchAction::Delete,
                        }),
                    )));
                    return;
                }
                _ => {}
            }
        }
        let disabled = match intent {
            ResetRegisterColumns => {
                if let Some(settings) = &mut self.settings {
                    settings.value_mut().register_columns.reset();
                }
                return;
            }
            ToggleInspector => {
                self.view.inspector_visible = !self.view.inspector_visible;
                return;
            }
            FocusSearch => {
                self.view.register_focus = Some(self.view.search_id);
                return;
            }
            NavigateOverview => {
                self.view.navigation.workspace = Workspace::Overview;
                return;
            }
            NavigateBudget => {
                self.view.navigation.workspace = Workspace::Budget;
                self.invalidations.insert(
                    crate::app::view_invalidation::ViewInvalidation::BudgetMonth(
                        self.view.selected_month,
                    ),
                );
                return;
            }
            NavigateCategories => {
                self.view.navigation.workspace = Workspace::Categories;
                self.request_category_catalog(None);
                return;
            }
            NavigateReports => {
                self.view.navigation.workspace = Workspace::Reports;
                return;
            }
            NavigateAllTransactions => {
                self.view.navigation.workspace = Workspace::AllTransactions;
                return;
            }
            // Budget widgets own their draft/preview state. These semantic intents exist so the
            // shared availability policy remains the sole source of enablement and explanations.
            AutoAssign | MoveMoney => {
                self.view.notifications.push(Notification::success(
                    if intent == AutoAssign {
                        "Auto-Assign preview ready"
                    } else {
                        "Move Money ready"
                    },
                    "Review the selected budget categories before confirming.",
                ));
                return;
            }
            CompleteOnboarding
                if self.database_lifecycle == DatabaseLifecycle::FirstRunRequired =>
            {
                let magnitude = match self.view.onboarding.parsed_opening_magnitude() {
                    Ok(value) => value,
                    Err(error) => {
                        self.startup_notice(NotificationKind::Error, "Check first account", error);
                        return;
                    }
                };
                let date = match self.view.onboarding.parsed_date() {
                    Ok(value) => value,
                    Err(error) => {
                        self.startup_notice(NotificationKind::Error, "Check first account", error);
                        return;
                    }
                };
                let Some(paths) = self.paths.as_ref() else {
                    return;
                };
                let wizard = self.view.onboarding.clone();
                let request = crate::service::onboarding_service::OnboardingRequest {
                    budget_name: wizard.budget_name,
                    account_name: wizard.account.name,
                    account_type: wizard.account.account_type,
                    opening_magnitude: magnitude,
                    balance_date: date,
                    group_name: wizard.account.group,
                    note: Some(wizard.account.note),
                    categories: wizard.selected_categories.into_iter().collect(),
                };
                match crate::service::onboarding_service::OnboardingService::initialize_database(
                    &paths.database,
                    request,
                    || {},
                ) {
                    Ok(initialized) => {
                        self.commit_session(initialized.session, initialized.worker);
                        self.database_lifecycle = DatabaseLifecycle::Ready;
                        self.view.dialog = None;
                        self.view.navigation.workspace = Workspace::Budget;
                        self.invalidations.insert(
                            crate::app::view_invalidation::ViewInvalidation::BudgetMonth(
                                self.view.selected_month,
                            ),
                        );
                    }
                    Err(error) => self.startup_notice(
                        NotificationKind::Error,
                        "Budget setup failed",
                        &error.to_string(),
                    ),
                }
                return;
            }
            Settings => {
                self.view.open_dialog(
                    Dialog::Settings,
                    egui::Id::new("settings-command"),
                    egui::Id::new("toolbar"),
                );
                return;
            }
            AddAccount => {
                self.view.editor = EditorState::CreatingAccount(AccountEditorState {
                    account_id: None,
                    form: Default::default(),
                    metadata: EditorMetadata::new(egui::Id::new("accounts")),
                });
                return;
            }
            EditAccount if let Some(id) = self.view.selected_account => {
                let form = self
                    .view
                    .accounts
                    .iter()
                    .find(|account| account.id == id)
                    .map_or_else(Default::default, |account| {
                        crate::ui::workspaces::all_accounts::AccountDialogForm {
                            name: account.name.clone(),
                            account_type: Some(account.account_type),
                            opening_magnitude: "0".into(),
                            opening_date: time::OffsetDateTime::now_utc().date().to_string(),
                            group_id: account.group_id,
                            note: String::new(),
                            favorite: account.favorite,
                        }
                    });
                self.view.editor = EditorState::EditingAccount(AccountEditorState {
                    account_id: Some(id),
                    form,
                    metadata: EditorMetadata::new(egui::Id::new("account-summary")),
                });
                return;
            }
            AddAccountGroup => {
                self.view.editor = EditorState::ManagingAccountGroup(GroupEditorState {
                    group_id: None,
                    metadata: EditorMetadata::new(egui::Id::new("account-groups")),
                });
                return;
            }
            AddTransaction if self.view.selected_account.is_some() => {
                self.view.editor = EditorState::CreatingTransaction(
                    crate::app::transaction_editor::TransactionEditorState::new(
                        self.view.selected_account,
                        EditorMetadata::new(egui::Id::new("register")),
                    ),
                );
                return;
            }
            EditTransaction if let Some(id) = self.view.selected_transaction => {
                self.view.editor = EditorState::EditingTransaction({
                    let mut e = crate::app::transaction_editor::TransactionEditorState::new(
                        self.view.selected_account,
                        EditorMetadata::new(egui::Id::new("register")),
                    );
                    e.transaction_id = Some(id);
                    e
                });
                return;
            }
            CreateTransfer if self.view.selected_account.is_some() => {
                self.view.editor = EditorState::CreatingTransfer(TransferEditorState {
                    draft: crate::ui::workspaces::register::TransferEditor {
                        from_account: self.view.selected_account,
                        ..Default::default()
                    },
                    metadata: EditorMetadata::new(egui::Id::new("register")),
                });
                return;
            }
            ReconcileAccount if self.view.selected_account.is_some() => {
                self.view.editor = EditorState::Reconciling(ReconciliationEditorState {
                    account_id: self.view.selected_account,
                    statement_balance: String::new(),
                    statement_date: String::new(),
                    metadata: EditorMetadata::new(egui::Id::new("register")),
                });
                return;
            }
            Import if self.view.selected_account.is_some() => {
                self.view.editor = EditorState::Importing(ImportEditorState {
                    account_id: self.view.selected_account,
                    source: std::path::PathBuf::new(),
                    batch_id: None,
                    metadata: EditorMetadata::new(egui::Id::new("register")),
                });
                return;
            }
            ContextualNew if self.view.selected_account.is_some() => {
                self.view.editor = EditorState::CreatingTransaction(
                    crate::app::transaction_editor::TransactionEditorState::new(
                        self.view.selected_account,
                        EditorMetadata::new(egui::Id::new("register")),
                    ),
                );
                return;
            }
            Commit if self.view.editor.is_active() => {
                self.commit_editor();
                return;
            }
            Cancel if self.view.editor.is_active() => {
                self.cancel_editor();
                return;
            }
            Cancel if self.view.dialog.is_some() => {
                self.view.dialog = None;
                return;
            }
            Exit => {
                self.request_exit();
                return;
            }
            CompleteOnboarding => "Onboarding is available only when the fixed database is absent",
            ContextualNew => "Select an account before creating a transaction",
            Import => "Select an account before importing transactions",
            Undo | Redo => unreachable!("history commands handled above"),
            Commit => "No editor is active",
            Cancel => "Nothing is open to cancel",
            Edit
            | Delete
            | Rename
            | ToggleSelection
            | SelectAllTransactions
            | EditAccount
            | CloseAccount
            | RenameAccountGroup
            | DeleteAccountGroup
            | MoveAccountGroup
            | EditTransaction
            | DeleteTransaction => "Select the corresponding transaction, account, or group first",
            AddTransaction | CreateTransfer | ReconcileAccount => "Select an active account first",
            MoveUp | MoveDown | NextField | PreviousField => {
                "Select an editable account group item first"
            }
            PreviousMonth | NextMonth if self.view.navigation.workspace == Workspace::Budget => {
                if let Some(month) = self.view.step_budget_month(intent == NextMonth) {
                    self.invalidations.insert(
                        crate::app::view_invalidation::ViewInvalidation::BudgetMonth(month),
                    );
                }
                return;
            }
            PreviousMonth | NextMonth => "Visit the Budget workspace to change its month",
            Backup if self.database_lifecycle == DatabaseLifecycle::Ready => {
                self.view.open_dialog(
                    Dialog::BudgetMaintenance,
                    egui::Id::new("backup-command"),
                    egui::Id::new("toolbar"),
                );
                return;
            }
            Backup => "Backup is available after the database finishes opening",
            RetryOperation => "No failed operation is selected",
            CancelOperation => "No cancellable operation is running",
        };
        self.view.notifications.push(Notification {
            kind: NotificationKind::Information,
            title: "Command unavailable".into(),
            detail: disabled.into(),
            persistent: false,
        });
    }

    fn dispatch_category(&mut self, intent: CategoryAction) {
        use crate::app::state::{
            CategoryEditorMode, CategoryEditorState, EditorMetadata, EditorState,
        };
        match intent {
            CategoryAction::RefreshCatalog => {
                self.request_category_catalog(self.view.register_focus)
            }
            CategoryAction::ToggleArchived(value) => {
                self.view.show_archived_categories = value;
                self.request_category_catalog(None);
            }
            CategoryAction::Select(id) => {
                self.view.selected_category = Some(id);
                self.request_category_detail(id, None);
            }
            CategoryAction::NewGroup => {
                self.view.editor = EditorState::ManagingCategory(CategoryEditorState {
                    category_id: None,
                    group_id: None,
                    name: String::new(),
                    mode: CategoryEditorMode::Group,
                    metadata: EditorMetadata::new(egui::Id::new("new-category-group")),
                })
            }
            CategoryAction::NewCategory(group_id) => {
                self.view.editor = EditorState::ManagingCategory(CategoryEditorState {
                    category_id: None,
                    group_id: Some(group_id),
                    name: String::new(),
                    mode: CategoryEditorMode::Category,
                    metadata: EditorMetadata::new(egui::Id::new("new-category")),
                })
            }
            CategoryAction::Edit(id) => {
                let name = self
                    .view
                    .category_detail
                    .last_successful
                    .as_ref()
                    .filter(|v| v.category.id == id)
                    .map_or_else(String::new, |v| v.category.name.clone());
                self.view.editor = EditorState::ManagingCategory(CategoryEditorState {
                    category_id: Some(id),
                    group_id: None,
                    name,
                    mode: CategoryEditorMode::Category,
                    metadata: EditorMetadata::new(egui::Id::new("category-detail")),
                });
            }
            CategoryAction::BeginGoal(id) => {
                self.view.editor = EditorState::ManagingCategory(CategoryEditorState {
                    category_id: Some(id),
                    group_id: None,
                    name: String::new(),
                    mode: CategoryEditorMode::Goal,
                    metadata: EditorMetadata::new(egui::Id::new("category-goal")),
                })
            }
            CategoryAction::OpenActivity(id) | CategoryAction::OpenTransactions(id) => {
                if let Some(budget_id) = self.view.active_budget {
                    self.view.register_query.active_request = Some(
                        crate::ui::workspaces::categories::canonical_category_filter(budget_id, id),
                    );
                    self.view.navigation.workspace =
                        crate::app::navigation::Workspace::AllTransactions;
                }
            }
            CategoryAction::BeginGoalTransfer(_) => self.dispatch_ui(AppCommand::CreateTransfer),
        }
    }

    fn request_category_catalog(&mut self, focus: Option<egui::Id>) {
        let Some(budget_id) = self.view.active_budget else {
            return;
        };
        let id = self.allocate_request();
        self.view.category_catalog.begin(id, self.generation, focus);
        let request = crate::storage::worker::StorageRequest {
            id,
            generation: self.generation,
            operation: crate::storage::worker::WorkerOperation::Category(
                crate::storage::worker::CategoryViewOperation::Catalog {
                    budget_id,
                    show_archived: self.view.show_archived_categories,
                },
            ),
        };
        if self
            .worker
            .as_ref()
            .is_none_or(|w| w.submit(request).is_err())
        {
            let _ = self.view.category_catalog.fail(
                id,
                self.generation,
                "Category catalog could not be requested.",
            );
        }
    }

    fn dispatch_report(&mut self, action: crate::app::command::ReportAction) {
        match action {
            crate::app::command::ReportAction::Refresh(request)
            | crate::app::command::ReportAction::Retry(request) => self.request_report(request),
            crate::app::command::ReportAction::ExportCsv { destination } => {
                let result = self
                    .view
                    .report_query
                    .view
                    .last_successful
                    .as_ref()
                    .ok_or_else(|| "There is no displayed report to export.".to_owned())
                    .and_then(|view| {
                        std::fs::write(&destination, view.csv.as_bytes()).map_err(|_| {
                            "The report could not be written to the selected destination."
                                .to_owned()
                        })
                    });
                let (kind, title, detail, persistent) = match result {
                    Ok(()) => (
                        NotificationKind::Information,
                        "Report exported",
                        destination.display().to_string(),
                        false,
                    ),
                    Err(message) => (NotificationKind::Error, "Export failed", message, true),
                };
                self.view.notifications.push(Notification {
                    kind,
                    title: title.into(),
                    detail,
                    persistent,
                });
            }
        }
    }

    fn request_report(&mut self, request: crate::domain::ReportRequest) {
        let Some(budget_id) = self.view.active_budget else {
            return;
        };
        let id = self.allocate_request();
        self.view
            .report_query
            .begin(request.clone(), id, self.generation);
        let operation = crate::storage::worker::WorkerOperation::Report(
            crate::storage::worker::ReportOperation { budget_id, request },
        );
        let submission = crate::storage::worker::StorageRequest {
            id,
            generation: self.generation,
            operation,
        };
        if self
            .worker
            .as_ref()
            .is_none_or(|worker| worker.submit(submission).is_err())
        {
            let _ = self.view.report_query.view.fail(
                id,
                self.generation,
                "The report could not be requested.",
            );
        }
    }
    fn request_category_detail(
        &mut self,
        category_id: crate::domain::CategoryId,
        focus: Option<egui::Id>,
    ) {
        let id = self.allocate_request();
        self.view.category_detail.begin(id, self.generation, focus);
        let today = time::OffsetDateTime::now_utc().date();
        let request = crate::storage::worker::StorageRequest {
            id,
            generation: self.generation,
            operation: crate::storage::worker::WorkerOperation::Category(
                crate::storage::worker::CategoryViewOperation::Detail { category_id, today },
            ),
        };
        if self
            .worker
            .as_ref()
            .is_none_or(|w| w.submit(request).is_err())
        {
            let _ = self.view.category_detail.fail(
                id,
                self.generation,
                "Category details could not be requested.",
            );
        }
    }
    fn handle_view_response(
        &mut self,
        id: RequestId,
        generation: Generation,
        result: Result<crate::storage::worker::TypedResult, crate::storage::worker::WorkerError>,
        safe: Option<crate::storage::worker::SafeUserError>,
    ) {
        match result {
            Ok(crate::storage::worker::TypedResult::AccountTree(value)) => {
                if self.view.account_tree.accept(id, generation, value.clone()) {
                    self.resolve_startup_destination(&value);
                }
            }
            Ok(crate::storage::worker::TypedResult::BudgetMonth(value)) => {
                if self.view.budget_month.accept(id, generation, value.clone()) {
                    self.view.budget_month_cache.insert(value);
                }
            }
            Ok(crate::storage::worker::TypedResult::InboxSummary(value)) => {
                let _ = self.view.inbox_summary.accept(id, generation, value);
            }
            Ok(crate::storage::worker::TypedResult::CategoryCatalog(value)) => {
                let _ = self.view.category_catalog.accept(id, generation, value);
            }
            Ok(crate::storage::worker::TypedResult::CategoryDetail(value)) => {
                let _ = self.view.category_detail.accept(id, generation, value);
            }
            Ok(crate::storage::worker::TypedResult::Report(value)) => {
                let _ = self.view.report_query.view.accept(id, generation, value);
            }
            Ok(crate::storage::worker::TypedResult::RegisterPage(value)) => {
                let _ = self.view.register_query.accept(id, generation, value);
            }
            Err(error) => {
                let message = safe.map_or_else(
                    || format!("Refresh failed: {error}"),
                    |v| v.rendered_message(),
                );
                let _ = self
                    .view
                    .category_catalog
                    .fail(id, generation, message.clone());
                let _ = self
                    .view
                    .category_detail
                    .fail(id, generation, message.clone());
                let _ = self
                    .view
                    .report_query
                    .view
                    .fail(id, generation, message.clone());
                let _ = self.view.account_tree.fail(id, generation, message.clone());
                let _ = self.view.budget_month.fail(id, generation, message.clone());
                let _ = self.view.inbox_summary.fail(id, generation, message);
            }
            _ => self.view.complete_request(id),
        }
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
                FailureSafety::NonRetryable(crate::storage::worker::SafeUserError::new(
                    "financial command",
                    "This operation belongs to a closed budget.",
                )),
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
                FailureSafety::Retryable(crate::storage::worker::SafeUserError::new(
                    "financial command",
                    "The operation could not be submitted. You may safely retry.",
                )),
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
        match result {
            Ok(crate::storage::worker::TypedResult::Mutation(m)) => {
                let success_title = match &c.envelope.payload {
                    ApplicationAction::Financial(FinancialCommand::Transaction(_)) => {
                        "Transaction saved"
                    }
                    ApplicationAction::Financial(FinancialCommand::Assignment(_)) => {
                        "Assignments updated"
                    }
                    _ => "Changes saved",
                };
                if self
                    .view
                    .editor
                    .metadata()
                    .is_some_and(|metadata| metadata.pending_command_id == Some(cid))
                {
                    if let Some(transaction_id) = m.affected_entity_ids.iter().find_map(|id| {
                        if let crate::storage::protocol::AffectedEntityId::Transaction(id) = id {
                            Some(*id)
                        } else {
                            None
                        }
                    }) {
                        self.view.register_selection.select_only(transaction_id);
                        self.view.selected_transaction = Some(transaction_id);
                    }
                }
                c.transition(CommandStatus::Committed)
                    .expect("running commits");
                self.terminal_sequence += 1;
                c.terminal_sequence = Some(self.terminal_sequence);
                if let Some(redo) = self.history_operations.remove(&cid) {
                    let subsequent = match &m.undo {
                        Some(crate::storage::protocol::UndoData::Command(command)) => {
                            Some(command.clone())
                        }
                        _ => None,
                    };
                    if redo {
                        let _ = self.history.redo();
                        if let Some(inverse) = subsequent {
                            self.history.replace_next_undo(inverse);
                        }
                    } else {
                        let _ = self.history.undo();
                        if let Some(command) = subsequent {
                            self.history.replace_next_redo(command);
                        }
                    }
                } else if let (
                    ApplicationAction::Financial(command),
                    Some(crate::storage::protocol::UndoData::Command(inverse)),
                ) = (&c.envelope.payload, m.undo)
                {
                    self.history.record_success(HistoryEntry {
                        label: if matches!(
                            command,
                            FinancialCommand::Transaction(TransactionCommand::Batch(_))
                        ) {
                            let count = m
                                .affected_entity_ids
                                .iter()
                                .filter(|id| {
                                    matches!(
                                        id,
                                        crate::storage::protocol::AffectedEntityId::Transaction(_)
                                    )
                                })
                                .count();
                            m.operation_label.strip_suffix(" transactions").map_or_else(
                                || format!("{} {count}", m.operation_label),
                                |verb| format!("{verb} {count} transactions"),
                            )
                        } else {
                            m.operation_label.clone()
                        },
                        command: command.clone(),
                        inverse,
                    });
                }
                self.invalidations.merge(m.invalidations);
                self.view
                    .notifications
                    .push(Notification::success(success_title, &m.operation_label));
            }
            Err(crate::storage::worker::WorkerError::Cancelled) => {
                let _ = c.transition(CommandStatus::Cancelled);
                self.terminal_sequence += 1;
                c.terminal_sequence = Some(self.terminal_sequence);
            }
            Err(_) => Self::fail_record(
                c,
                FailureSafety::Retryable(safe.unwrap_or(
                    crate::storage::worker::SafeUserError::new(
                        "storage",
                        "The operation failed without changing your data. You may safely retry.",
                    ),
                )),
                &mut self.terminal_sequence,
            ),
            _ => Self::fail_record(
                c,
                FailureSafety::NonRetryable(crate::storage::worker::SafeUserError::new(
                    "storage",
                    "The worker returned an unexpected result.",
                )),
                &mut self.terminal_sequence,
            ),
        }
        if let Some(command) = self.pending_commands.get(&cid)
            && command.status == CommandStatus::Failed
        {
            let detail = command.safe_failure.as_ref().map_or_else(
                || "The operation failed without changing your data.".to_owned(),
                |failure| format!("{failure:?}"),
            );
            self.view
                .notifications
                .push(Notification::actionable_error("Operation failed", detail));
        }
        let editor_owns_response = self
            .view
            .editor
            .metadata()
            .is_some_and(|metadata| metadata.pending_command_id == Some(cid));
        let (editor_result, editor_error) =
            self.pending_commands
                .get(&cid)
                .map_or((None, None), |command| {
                    (
                        editor_owns_response.then_some(command.status),
                        command
                            .safe_failure
                            .as_ref()
                            .map(|error| format!("{error:?}")),
                    )
                });
        self.prune_commands();
        match editor_result {
            Some(CommandStatus::Committed) => self.cancel_editor(),
            Some(CommandStatus::Failed) => self.fail_editor(editor_error.unwrap_or_else(|| {
                "The operation failed without changing your data. You may retry.".into()
            })),
            _ => {}
        }
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
        self.history.clear();
        self.history_operations.clear();
        let effects = self.lifecycle.native_close_requested();
        self.lifecycle_effects.extend(effects);
        self.advance_exit_review();
    }
    pub fn request_exit(&mut self) {
        self.history.clear();
        self.history_operations.clear();
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
                    self.view.mutations_disabled = true;
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
                fixed_database_exists: false,
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
        for _ in 0..50 {
            runtime.drain_worker_responses();
            if !runtime.view.account_tree.refresh_active
                && !runtime.view.budget_month.refresh_active
                && !runtime.view.category_catalog.refresh_active
                && !runtime.view.inbox_summary.refresh_active
            {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        (dir, runtime)
    }

    #[test]
    fn hydration_is_scheduled_once_per_session() {
        let (_dir, mut runtime) = runtime_with_worker();
        let next = runtime.next_request;
        runtime.hydrate_session();
        runtime.hydrate_session();
        assert_eq!(runtime.next_request, next);
        assert!(runtime.view.account_tree.last_successful.is_some());
        assert!(runtime.view.budget_month.last_successful.is_some());
        assert!(!runtime.view.category_catalog.refresh_active);
        assert!(runtime.view.inbox_summary.last_successful.is_some());
    }

    #[test]
    fn stale_month_response_is_ignored_and_failure_is_visible() {
        let (_dir, mut runtime) = runtime_with_worker();
        let generation = runtime.generation;
        let current_id = runtime.allocate_request();
        runtime
            .view
            .budget_month
            .begin(current_id, generation, None);
        let month = runtime.view.selected_month;
        let stale = crate::app::view_model::BudgetMonthView {
            version: crate::app::view_model::ViewVersion {
                generation: generation.budget,
                revision: 1,
            },
            month,
            calculation_revision: 1,
            ready_to_assign_cents: 1,
            assigned_cents: 0,
            activity_cents: 0,
            available_cents: 0,
            overspending_cents: 0,
            cash_overspending_cents: 0,
            credit_card_overspending_cents: 0,
            rows: vec![],
            inspector: vec![],
        };
        runtime.handle_view_response(
            current_id - 1,
            generation,
            Ok(crate::storage::worker::TypedResult::BudgetMonth(stale)),
            None,
        );
        assert!(runtime.view.budget_month.refresh_active);
        runtime.handle_view_response(
            current_id,
            generation,
            Err(crate::storage::worker::WorkerError::Repository(
                "offline".into(),
            )),
            None,
        );
        assert!(
            runtime
                .view
                .budget_month
                .safe_failure
                .as_deref()
                .is_some_and(|message| message.contains("offline"))
        );
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
                    operation_label: "test".into(),
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
                fixed_database_exists: false,
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
                fixed_database_exists: false,
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
                .prepare_fixed(&paths, || {})
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
                fixed_database_exists: false,
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
            fixed_database_exists: false,
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
    fn missing_fixed_database_enters_first_run() {
        let dir = tempfile::tempdir().unwrap();
        let paths = PortablePaths::from_executable(&dir.path().join("mnab.exe")).unwrap();
        let runtime = ApplicationRuntime::new(
            Some(paths),
            None,
            false,
            StartupContext {
                marker_was_absent: false,
                fixed_database_exists: false,
            },
        );
        assert_eq!(
            runtime.database_lifecycle(),
            DatabaseLifecycle::FirstRunRequired
        );
        assert!(runtime.session().is_none());
    }

    #[test]
    fn corrupt_fixed_database_enters_recovery() {
        let dir = tempfile::tempdir().unwrap();
        let paths = PortablePaths::from_executable(&dir.path().join("mnab.exe")).unwrap();
        std::fs::write(&paths.database, b"not sqlite").unwrap();
        let runtime = ApplicationRuntime::new(
            Some(paths),
            None,
            false,
            StartupContext {
                marker_was_absent: false,
                fixed_database_exists: true,
            },
        );
        assert_eq!(
            runtime.database_lifecycle(),
            DatabaseLifecycle::RecoveryRequired
        );
        assert!(runtime.session().is_none());
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
                fixed_database_exists: false,
            },
        );
        assert!(runtime.shutdown().is_err());
        assert!(!paths.data.join(".clean-shutdown").exists());
    }

    fn valid_account_editor(runtime: &mut ApplicationRuntime) {
        runtime.dispatch(ApplicationAction::Ui(AppCommand::AddAccount));
        let crate::app::state::EditorState::CreatingAccount(editor) = &mut runtime.view.editor
        else {
            panic!("account editor should be active");
        };
        editor.form.name = "Checking".into();
        editor.form.account_type = Some(crate::domain::AccountType::Checking);
        editor.form.opening_magnitude = "0".into();
        editor.form.opening_date = "2026-08-09".into();
    }

    fn finish_editor_command(
        runtime: &mut ApplicationRuntime,
        result: Result<crate::storage::worker::TypedResult, crate::storage::worker::WorkerError>,
        user_error: Option<crate::storage::worker::SafeUserError>,
    ) {
        let command_id = runtime
            .view
            .editor
            .metadata()
            .and_then(|metadata| metadata.pending_command_id)
            .expect("editor should own a command");
        let command = runtime.pending_commands[&command_id].clone();
        runtime.handle_command_response(
            command.worker_request_id.unwrap(),
            command_id,
            command.envelope.correlation_id,
            runtime.generation,
            result,
            user_error,
        );
    }

    fn successful_mutation(
        command_id: u64,
        correlation_id: u64,
    ) -> crate::storage::worker::TypedResult {
        crate::storage::worker::TypedResult::Mutation(crate::storage::protocol::MutationResult {
            command_id,
            correlation_id,
            operation_label: "Save account".into(),
            affected_entity_ids: vec![],
            undo: None,
            invalidations: Default::default(),
            navigation: None,
            focus_restoration: None,
            notice: None,
        })
    }

    #[test]
    fn cancelling_editor_discards_draft_and_restores_focus() {
        let mut runtime = ApplicationRuntime::new(
            None,
            None,
            false,
            StartupContext {
                marker_was_absent: false,
                fixed_database_exists: false,
            },
        );
        runtime.dispatch(ApplicationAction::Ui(AppCommand::AddAccount));
        runtime.dispatch(ApplicationAction::Ui(AppCommand::Cancel));
        assert!(matches!(
            runtime.view.editor,
            crate::app::state::EditorState::Idle
        ));
        assert_eq!(runtime.view.register_focus, Some(egui::Id::new("accounts")));
    }

    #[test]
    fn validation_failure_retains_input_and_allows_retry_or_cancel() {
        let (_dir, mut runtime) = runtime_with_worker();
        runtime.dispatch(ApplicationAction::Ui(AppCommand::AddAccount));
        runtime.dispatch(ApplicationAction::Ui(AppCommand::Commit));
        let crate::app::state::EditorState::CreatingAccount(editor) = &runtime.view.editor else {
            panic!("invalid editor must remain mounted");
        };
        assert_eq!(editor.form.name, "");
        assert_eq!(
            editor.metadata.commit_state,
            crate::app::state::CommitState::Failed
        );
        assert!(!editor.metadata.validation_errors.is_empty());
        assert!(runtime.pending_commands.is_empty());

        runtime.dispatch(ApplicationAction::Ui(AppCommand::Cancel));
        assert!(matches!(
            runtime.view.editor,
            crate::app::state::EditorState::Idle
        ));
        assert_eq!(runtime.view.register_focus, Some(egui::Id::new("accounts")));
    }

    #[test]
    fn commit_submits_once_and_closes_only_after_worker_success() {
        let (_dir, mut runtime) = runtime_with_worker();
        valid_account_editor(&mut runtime);
        runtime.dispatch(ApplicationAction::Ui(AppCommand::Commit));
        let metadata = runtime.view.editor.metadata().unwrap();
        assert_eq!(
            metadata.commit_state,
            crate::app::state::CommitState::Submitting
        );
        let command_id = metadata.pending_command_id.unwrap();
        let correlation_id = runtime.pending_commands[&command_id]
            .envelope
            .correlation_id;
        assert_eq!(runtime.pending_commands.len(), 1);

        runtime.dispatch(ApplicationAction::Ui(AppCommand::Commit));
        assert_eq!(
            runtime.pending_commands.len(),
            1,
            "duplicate commit was ignored"
        );
        assert!(
            runtime.view.editor.is_active(),
            "editor stays open while submitting"
        );

        finish_editor_command(
            &mut runtime,
            Ok(successful_mutation(command_id, correlation_id)),
            None,
        );
        assert!(matches!(
            runtime.view.editor,
            crate::app::state::EditorState::Idle
        ));
        assert_eq!(runtime.view.register_focus, Some(egui::Id::new("accounts")));
    }

    #[test]
    fn worker_failure_retains_draft_and_permits_retry() {
        let (_dir, mut runtime) = runtime_with_worker();
        valid_account_editor(&mut runtime);
        runtime.dispatch(ApplicationAction::Ui(AppCommand::Commit));
        finish_editor_command(
            &mut runtime,
            Err(crate::storage::worker::WorkerError::Repository(
                "disk full".into(),
            )),
            Some(crate::storage::worker::SafeUserError::new(
                "storage",
                "Saving failed without changing data. Free disk space and retry.",
            )),
        );
        let crate::app::state::EditorState::CreatingAccount(editor) = &runtime.view.editor else {
            panic!("failed editor must remain mounted");
        };
        assert_eq!(editor.form.name, "Checking");
        assert_eq!(
            editor.metadata.commit_state,
            crate::app::state::CommitState::Failed
        );
        assert!(editor.metadata.pending_command_id.is_none());
        assert!(editor.metadata.validation_errors[0].contains("retry"));

        let before = runtime.pending_commands.len();
        runtime.dispatch(ApplicationAction::Ui(AppCommand::Commit));
        assert_eq!(
            runtime.view.editor.metadata().unwrap().commit_state,
            crate::app::state::CommitState::Submitting
        );
        assert!(runtime.pending_commands.len() > before);
    }

    #[test]
    fn changing_accounts_invalidates_account_projection() {
        let mut runtime = ApplicationRuntime::new(
            None,
            None,
            false,
            StartupContext {
                marker_was_absent: false,
                fixed_database_exists: false,
            },
        );
        let before = runtime.generation.view;
        let id = AccountId::new();
        runtime.select_account(id);
        assert!(runtime.generation.view > before);
        assert!(runtime.invalidations.iter().any(|v| *v == crate::app::view_invalidation::ViewInvalidation::AccountRegister(id)));
    }
}
