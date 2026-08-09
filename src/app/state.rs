use crate::{
    app::{
        message::{WorkerMessage, WorkerPayload},
        navigation::Navigation,
    },
    domain::{
        AccountGroupId, AccountId, BudgetId, BudgetMonth, CategoryGroupId, CategoryId,
        ImportBatchId, Money, TransactionId,
    },
    storage::worker::{Generation, RequestId},
};
use egui::Id;
use std::{collections::BTreeMap, path::PathBuf};

/// State shared by every independently refreshed immutable projection.
#[derive(Clone, Debug)]
pub struct ViewQueryState<T> {
    pub last_successful: Option<T>,
    pub refresh_active: bool,
    pub latest_request_id: Option<RequestId>,
    pub latest_generation: Generation,
    pub safe_failure: Option<String>,
    /// Focus is restored only for the accepted latest response, never merely because data arrived.
    pub preserve_focus: Option<Id>,
}
impl<T> Default for ViewQueryState<T> {
    fn default() -> Self {
        Self {
            last_successful: None,
            refresh_active: false,
            latest_request_id: None,
            latest_generation: Generation { budget: 0, view: 0 },
            safe_failure: None,
            preserve_focus: None,
        }
    }
}
impl<T> ViewQueryState<T> {
    pub fn begin(&mut self, id: RequestId, generation: Generation, focus: Option<Id>) {
        self.refresh_active = true;
        self.latest_request_id = Some(id);
        self.latest_generation = generation;
        self.safe_failure = None;
        self.preserve_focus = focus;
    }
    #[must_use]
    pub fn accept(&mut self, id: RequestId, generation: Generation, value: T) -> bool {
        if self.latest_request_id != Some(id) || self.latest_generation != generation {
            return false;
        }
        self.last_successful = Some(value);
        self.refresh_active = false;
        self.safe_failure = None;
        true
    }
    #[must_use]
    pub fn fail(
        &mut self,
        id: RequestId,
        generation: Generation,
        safe_failure: impl Into<String>,
    ) -> bool {
        if self.latest_request_id != Some(id) || self.latest_generation != generation {
            return false;
        }
        self.refresh_active = false;
        self.safe_failure = Some(safe_failure.into());
        true
    }
}

/// Correlated report projection plus the exact request needed for retry/invalidation.
#[derive(Clone, Debug, Default)]
pub struct ReportQueryState {
    pub current_request: Option<crate::domain::ReportRequest>,
    pub view: ViewQueryState<crate::app::view_model::ReportView>,
}

impl ReportQueryState {
    pub fn begin(
        &mut self,
        request: crate::domain::ReportRequest,
        id: RequestId,
        generation: Generation,
    ) {
        self.current_request = Some(request);
        self.view.begin(id, generation, None);
    }
}

/// Correlated register paging state. A failed refresh deliberately leaves
/// `last_successful` intact so the ledger never disappears behind an error.
#[derive(Clone, Debug, Default)]
pub struct RegisterQueryState {
    pub last_successful: Option<crate::app::view_model::RegisterPageView>,
    pub active_request: Option<crate::app::view_model::RegisterRequest>,
    pub refresh_active: bool,
    pub next_page_active: bool,
    pub latest_request_id: Option<RequestId>,
    pub latest_generation: Generation,
    pub safe_failure: Option<String>,
}
impl RegisterQueryState {
    pub fn begin(
        &mut self,
        id: RequestId,
        generation: Generation,
        request: crate::app::view_model::RegisterRequest,
        next_page: bool,
    ) {
        self.latest_request_id = Some(id);
        self.latest_generation = generation;
        self.active_request = Some(request);
        self.next_page_active = next_page;
        self.refresh_active = !next_page;
        self.safe_failure = None;
    }
    pub fn accept(
        &mut self,
        id: RequestId,
        generation: Generation,
        page: crate::app::view_model::RegisterPageView,
    ) -> bool {
        if self.latest_request_id != Some(id)
            || self.latest_generation != generation
            || self.active_request.as_ref() != Some(&page.request)
        {
            return false;
        }
        if self.next_page_active {
            let Some(current) = self.last_successful.as_mut() else {
                return false;
            };
            if current.next_cursor != page.cursor {
                return false;
            }
            let known = current
                .rows
                .iter()
                .map(|r| r.transaction_id)
                .collect::<std::collections::BTreeSet<_>>();
            if page.rows.iter().any(|r| known.contains(&r.transaction_id)) {
                return false;
            }
            current.rows.extend(page.rows);
            current.next_cursor = page.next_cursor;
            current.has_more = page.has_more;
            current.total_matches = page.total_matches;
        } else {
            self.last_successful = Some(page);
        }
        self.refresh_active = false;
        self.next_page_active = false;
        self.safe_failure = None;
        true
    }
    pub fn fail(
        &mut self,
        id: RequestId,
        generation: Generation,
        error: impl Into<String>,
    ) -> bool {
        if self.latest_request_id != Some(id) || self.latest_generation != generation {
            return false;
        }
        self.refresh_active = false;
        self.next_page_active = false;
        self.safe_failure = Some(error.into());
        true
    }
}

#[derive(Clone, Debug)]
pub struct AccountSummary {
    pub id: AccountId,
    pub name: String,
    pub working_balance: Money,
    pub unreconciled: bool,
    pub tracking: bool,
    pub closed: bool,
    pub group_id: Option<AccountGroupId>,
    pub favorite: bool,
    pub cleared_balance: Money,
    pub account_type: crate::domain::AccountType,
}
#[derive(Clone, Debug)]
pub enum Dialog {
    Onboarding,
    BudgetMaintenance,
    RepairBudget,
    RecoveryChoice,
    Reconcile(AccountId),
    Import(AccountId),
    Settings,
}
#[derive(Clone, Debug)]
pub enum InspectorContext {
    AccountSummary(Option<AccountId>),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommitState {
    Editing,
    Validating,
    Submitting,
    Failed,
}

#[derive(Clone, Debug)]
pub struct EditorMetadata {
    pub validation_errors: Vec<String>,
    pub dirty: bool,
    pub commit_state: CommitState,
    pub restore_focus: Id,
    /// Runtime command owning the in-flight submission.  Keeping this on the
    /// editor makes worker completion correlation explicit and prevents a
    /// second press of Commit from enqueueing duplicate work.
    pub pending_command_id: Option<u64>,
    pub pending_request_id: Option<RequestId>,
    pub pending_generation: Option<Generation>,
}
impl EditorMetadata {
    pub fn new(restore_focus: Id) -> Self {
        Self {
            validation_errors: vec![],
            dirty: false,
            commit_state: CommitState::Editing,
            restore_focus,
            pending_command_id: None,
            pending_request_id: None,
            pending_generation: None,
        }
    }

    /// Starts a commit attempt unless this editor already owns an attempt.
    /// Failed attempts are intentionally retryable and keep the same form data.
    pub fn begin_validation(&mut self) -> bool {
        if matches!(
            self.commit_state,
            CommitState::Validating | CommitState::Submitting
        ) {
            return false;
        }
        self.commit_state = CommitState::Validating;
        self.pending_command_id = None;
        self.pending_request_id = None;
        self.pending_generation = None;
        self.validation_errors.clear();
        true
    }

    pub fn begin_submission(&mut self, command_id: u64) {
        self.commit_state = CommitState::Submitting;
        self.pending_command_id = Some(command_id);
    }

    pub fn fail(&mut self, error: impl Into<String>) {
        self.commit_state = CommitState::Failed;
        self.pending_command_id = None;
        self.pending_request_id = None;
        self.pending_generation = None;
        self.validation_errors = vec![error.into()];
    }
}
#[derive(Clone, Debug)]
pub struct AccountEditorState {
    pub account_id: Option<AccountId>,
    pub form: crate::ui::workspaces::all_accounts::AccountDialogForm,
    pub metadata: EditorMetadata,
}
#[derive(Clone, Debug)]
pub struct TransferEditorState {
    pub draft: crate::ui::workspaces::register::TransferEditor,
    /// Present when the workflow edits an existing linked transfer.  Keeping this
    /// identity prevents an edit from silently becoming a new transfer/categorized row.
    pub transfer_id: Option<crate::domain::TransferId>,
    pub edited_leg_id: Option<crate::domain::TransactionId>,
    pub other_leg_id: Option<crate::domain::TransactionId>,
    pub metadata: EditorMetadata,
}
#[derive(Clone, Debug)]
pub struct ImportEditorState {
    pub account_id: Option<AccountId>,
    pub source: PathBuf,
    pub batch_id: Option<ImportBatchId>,
    pub metadata: EditorMetadata,
}
#[derive(Clone, Debug)]
pub struct ReconciliationEditorState {
    pub account_id: Option<AccountId>,
    pub statement_balance: String,
    pub statement_date: String,
    pub metadata: EditorMetadata,
}
#[derive(Clone, Debug)]
pub struct CategoryEditorState {
    pub category_id: Option<CategoryId>,
    pub group_id: Option<CategoryGroupId>,
    pub name: String,
    pub mode: CategoryEditorMode,
    pub metadata: EditorMetadata,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CategoryEditorMode {
    Group,
    Category,
    Goal,
}
#[derive(Clone, Debug)]
pub struct GroupEditorState {
    pub group_id: Option<AccountGroupId>,
    pub metadata: EditorMetadata,
}

#[derive(Clone, Debug, Default)]
pub enum EditorState {
    #[default]
    Idle,
    CreatingAccount(AccountEditorState),
    EditingAccount(AccountEditorState),
    CreatingTransaction(crate::app::transaction_editor::TransactionEditorState),
    EditingTransaction(crate::app::transaction_editor::TransactionEditorState),
    CreatingTransfer(TransferEditorState),
    Importing(ImportEditorState),
    Reconciling(ReconciliationEditorState),
    ManagingCategory(CategoryEditorState),
    ManagingAccountGroup(GroupEditorState),
}
impl EditorState {
    pub fn metadata(&self) -> Option<&EditorMetadata> {
        match self {
            Self::Idle => None,
            Self::CreatingAccount(x) | Self::EditingAccount(x) => Some(&x.metadata),
            Self::CreatingTransaction(x) | Self::EditingTransaction(x) => Some(&x.metadata),
            Self::CreatingTransfer(x) => Some(&x.metadata),
            Self::Importing(x) => Some(&x.metadata),
            Self::Reconciling(x) => Some(&x.metadata),
            Self::ManagingCategory(x) => Some(&x.metadata),
            Self::ManagingAccountGroup(x) => Some(&x.metadata),
        }
    }
    pub fn metadata_mut(&mut self) -> Option<&mut EditorMetadata> {
        match self {
            Self::Idle => None,
            Self::CreatingAccount(x) | Self::EditingAccount(x) => Some(&mut x.metadata),
            Self::CreatingTransaction(x) | Self::EditingTransaction(x) => Some(&mut x.metadata),
            Self::CreatingTransfer(x) => Some(&mut x.metadata),
            Self::Importing(x) => Some(&mut x.metadata),
            Self::Reconciling(x) => Some(&mut x.metadata),
            Self::ManagingCategory(x) => Some(&mut x.metadata),
            Self::ManagingAccountGroup(x) => Some(&mut x.metadata),
        }
    }
    pub fn is_active(&self) -> bool {
        !matches!(self, Self::Idle)
    }
}
#[derive(Clone, Debug)]
pub struct Notification {
    pub kind: NotificationKind,
    pub title: String,
    pub detail: String,
    pub persistent: bool,
}
impl Notification {
    #[must_use]
    pub fn success(title: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            kind: NotificationKind::Information,
            title: title.into(),
            detail: detail.into(),
            persistent: false,
        }
    }

    #[must_use]
    pub fn actionable_error(title: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            kind: NotificationKind::Error,
            title: title.into(),
            detail: detail.into(),
            persistent: true,
        }
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NotificationKind {
    Information,
    Warning,
    Error,
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum RequestPurpose {
    Accounts,
    BudgetMonth(BudgetMonth),
    AccountRegister(AccountId),
    AllAccountRegisters,
    Inbox,
    Reports,
    Targets,
    Schedules,
    Search,
    Inspector,
}
#[derive(Clone, Debug)]
pub enum OperationStatus {
    Running { progress: Option<u8> },
    Failed(crate::app::message::OperationError),
}
#[derive(Clone, Debug)]
pub struct BackgroundOperation {
    pub request_id: RequestId,
    pub generation: Generation,
    pub label: String,
    pub status: OperationStatus,
}
#[derive(Clone, Debug)]
pub struct DialogState {
    pub dialog: Dialog,
    pub restore_focus: Id,
    pub fallback_container: Id,
}

#[derive(Debug)]
pub struct AppState {
    pub onboarding: crate::ui::onboarding::OnboardingWizard,
    /// Summary and bounded detail are independent projections/requests.
    pub inbox_counts: crate::app::inbox::InboxCounts,
    pub inbox_review: Vec<crate::app::inbox::InboxItem>,
    pub active_budget: Option<BudgetId>,
    pub budget_name: String,
    pub database_path: Option<PathBuf>,
    pub navigation: Navigation,
    pub selected_account: Option<AccountId>,
    pub selected_transaction: Option<TransactionId>,
    pub register_selection: crate::app::register::TransactionSelection,
    pub register_focus: Option<Id>,
    pub editor: EditorState,
    pub selected_month: BudgetMonth,
    pub report_query: ReportQueryState,
    pub dialog: Option<DialogState>,
    /// Risky data action retained only while its confirmation preview is visible.
    pub pending_data_action: Option<crate::app::command::DataAction>,
    pub maintenance_budget_name: String,
    pub notifications: Vec<Notification>,
    pub operations: BTreeMap<RequestId, BackgroundOperation>,
    latest_by_purpose: BTreeMap<RequestPurpose, RequestId>,
    purpose_by_request: BTreeMap<RequestId, RequestPurpose>,
    pub generation: Generation,
    pub inspector_context: InspectorContext,
    pub inspector_visible: bool,
    pub sidebar_width: f32,
    pub inspector_width: f32,
    pub accounts: Vec<AccountSummary>,
    pub account_groups: Vec<crate::domain::AccountGroup>,
    pub account_tree: ViewQueryState<Vec<crate::storage::query_store::AccountTreeGroup>>,
    pub budget_month: ViewQueryState<crate::app::view_model::BudgetMonthView>,
    pub budget_month_cache: crate::app::view_model::BudgetMonthCache,
    pub budget_ui: crate::ui::budget_view::BudgetUiState,
    pub inbox_summary: ViewQueryState<crate::app::view_model::InboxSummaryView>,
    pub register_query: RegisterQueryState,
    pub category_catalog: ViewQueryState<crate::app::view_model::CategoryCatalogView>,
    pub category_detail: ViewQueryState<crate::app::view_model::CategoryDetailView>,
    pub selected_category: Option<CategoryId>,
    pub show_archived_categories: bool,
    pub search: String,
    pub search_id: Id,
    pub palette: crate::app::palette::PaletteState,
    pub palette_shortcut: String,
    pub mutations_disabled: bool,
    pub can_undo: bool,
    pub can_redo: bool,
    pub undo_label: Option<String>,
    pub redo_label: Option<String>,
}
impl Default for AppState {
    fn default() -> Self {
        let nav = Navigation::default();
        let now =
            time::OffsetDateTime::now_local().unwrap_or_else(|_| time::OffsetDateTime::now_utc());
        Self {
            onboarding: crate::ui::onboarding::OnboardingWizard::default(),
            inbox_counts: crate::app::inbox::InboxCounts::default(),
            inbox_review: vec![],
            active_budget: None,
            budget_name: "No budget open".into(),
            database_path: None,
            navigation: nav,
            selected_account: None,
            selected_transaction: None,
            register_selection: Default::default(),
            register_focus: None,
            editor: EditorState::Idle,
            selected_month: BudgetMonth::new(now.year(), u8::from(now.month()))
                .expect("current calendar month is valid"),
            report_query: ReportQueryState::default(),
            dialog: None,
            pending_data_action: None,
            maintenance_budget_name: String::new(),
            notifications: vec![],
            operations: BTreeMap::new(),
            latest_by_purpose: BTreeMap::new(),
            purpose_by_request: BTreeMap::new(),
            generation: Generation { budget: 0, view: 0 },
            inspector_context: InspectorContext::AccountSummary(None),
            inspector_visible: true,
            sidebar_width: 230.0,
            inspector_width: 280.0,
            accounts: vec![],
            account_groups: vec![],
            account_tree: ViewQueryState::default(),
            budget_month: ViewQueryState::default(),
            budget_month_cache: Default::default(),
            budget_ui: Default::default(),
            inbox_summary: ViewQueryState::default(),
            register_query: RegisterQueryState::default(),
            category_catalog: ViewQueryState::default(),
            category_detail: ViewQueryState::default(),
            selected_category: None,
            show_archived_categories: false,
            search: String::new(),
            search_id: Id::new("global-search"),
            palette: crate::app::palette::PaletteState::default(),
            palette_shortcut: crate::app::settings::DEFAULT_PALETTE_SHORTCUT.into(),
            mutations_disabled: false,
            can_undo: false,
            can_redo: false,
            undo_label: None,
            redo_label: None,
        }
    }
}
impl AppState {
    /// Transient notices may be dismissed by the shell; persistent failures require an explicit
    /// user dismissal so retry/details remain reachable.
    pub fn dismiss_notification(&mut self, index: usize) -> bool {
        if index < self.notifications.len() {
            self.notifications.remove(index);
            true
        } else {
            false
        }
    }

    /// One shared snapshot for toolbar, workspace, overview, and palette action gating.
    #[must_use]
    pub fn action_context(&self) -> crate::app::command::CommandAvailabilityContext {
        use crate::app::command::CommandWorkspace;
        use crate::app::navigation::Workspace;
        crate::app::command::CommandAvailabilityContext {
            database_available: self.active_budget.is_some(),
            workspace: match self.navigation.workspace {
                Workspace::Overview => CommandWorkspace::Overview,
                Workspace::Budget => CommandWorkspace::Budget,
                Workspace::Categories => CommandWorkspace::Categories,
                Workspace::Reports => CommandWorkspace::Reports,
                Workspace::AllTransactions => CommandWorkspace::AllTransactions,
                Workspace::Inbox => CommandWorkspace::Inbox,
                Workspace::Account(_) => CommandWorkspace::AccountRegister,
            },
            has_selection: self.selected_account.is_some() || self.selected_transaction.is_some(),
            editing: self.editor.is_active(),
            dialog_open: self.dialog.is_some(),
            read_only: self.mutations_disabled,
            mutation_locked: !self.operations.is_empty(),
            can_undo: self.can_undo,
            can_redo: self.can_redo,
            selected_account: self.selected_account.is_some(),
            selected_transaction: self.selected_transaction.is_some(),
            selected_reconciled_transaction: self
                .register_query
                .last_successful
                .as_ref()
                .is_some_and(|page| {
                    page.rows.iter().any(|row| {
                        self.register_selection.contains(row.transaction_id) && row.reconciled
                    })
                }),
            budget_selection: !self.budget_ui.selected_categories.is_empty(),
            auto_assign_selection: self
                .budget_ui
                .auto_preview
                .as_ref()
                .is_some_and(|preview| !preview.changes.is_empty()),
            editor_valid: self
                .editor
                .metadata()
                .is_none_or(|metadata| metadata.validation_errors.is_empty()),
            ..Default::default()
        }
    }
    /// Changes the planning month only while the month-aware Budget workspace is active.
    /// Returns the new month when callers should request its projection.
    pub fn step_budget_month(&mut self, forward: bool) -> Option<BudgetMonth> {
        if self.navigation.workspace != crate::app::navigation::Workspace::Budget {
            return None;
        }
        let month = if forward {
            self.selected_month.next()
        } else {
            self.selected_month.previous()
        }
        .ok()?;
        self.selected_month = month;
        Some(month)
    }

    /// Applies all parts of register navigation as one state transition. The returned
    /// first-page request is the only request the caller should submit.
    pub fn open_register(
        &mut self,
        destination: &crate::app::navigation::RegisterDestination,
    ) -> Option<crate::app::view_model::RegisterRequest> {
        let budget_id = self.active_budget?;
        self.navigation.workspace = match destination.scope {
            crate::app::view_model::RegisterScope::Account(id) => {
                crate::app::navigation::Workspace::Account(id)
            }
            crate::app::view_model::RegisterScope::AllTransactions => {
                crate::app::navigation::Workspace::AllTransactions
            }
        };
        self.selected_account = match destination.scope {
            crate::app::view_model::RegisterScope::Account(id) => Some(id),
            _ => None,
        };
        self.selected_transaction = None;
        self.register_selection.clear();
        self.editor = EditorState::Idle;
        self.generation.view = self.generation.view.saturating_add(1);
        let request =
            destination.request(budget_id, crate::app::view_model::MAX_REGISTER_PAGE_SIZE);
        self.register_query.active_request = Some(request.clone());
        self.register_query.last_successful = None;
        Some(request)
    }
    /// Remove state whose identity belongs to the previous budget.
    pub fn clear_budget_state(&mut self) {
        self.active_budget = None;
        self.budget_name = "No budget open".into();
        self.database_path = None;
        self.selected_account = None;
        self.selected_transaction = None;
        self.register_selection.clear();
        self.editor = EditorState::Idle;
        self.report_query = ReportQueryState::default();
        self.accounts.clear();
        self.account_groups.clear();
        self.account_tree = ViewQueryState::default();
        self.budget_month = ViewQueryState::default();
        self.budget_month_cache = Default::default();
        self.budget_ui = Default::default();
        self.inbox_summary = ViewQueryState::default();
        self.operations.clear();
        self.latest_by_purpose.clear();
        self.purpose_by_request.clear();
        self.register_query = RegisterQueryState::default();
    }

    pub fn open_dialog(&mut self, dialog: Dialog, initiating: Id, fallback: Id) {
        self.dialog = Some(DialogState {
            dialog,
            restore_focus: initiating,
            fallback_container: fallback,
        });
    }
    pub fn close_dialog(&mut self, ctx: &egui::Context, existing: impl Fn(Id) -> bool) {
        if let Some(d) = self.dialog.take() {
            ctx.memory_mut(|m| {
                m.request_focus(if existing(d.restore_focus) {
                    d.restore_focus
                } else {
                    d.fallback_container
                })
            });
        }
    }
    /// Applies only correlated responses; stale work cannot alter the current view.
    pub fn apply_worker_message(&mut self, message: WorkerMessage) -> bool {
        let Some(purpose) = self.purpose_by_request.get(&message.request_id) else {
            return false;
        };
        if self.latest_by_purpose.get(purpose) != Some(&message.request_id)
            || message.generation != self.generation
        {
            return false;
        }
        if let Some(op) = self.operations.get_mut(&message.request_id) {
            op.status = match message.payload {
                WorkerPayload::Progress(p) => OperationStatus::Running { progress: Some(p) },
                WorkerPayload::Failed(e) => OperationStatus::Failed(e),
                WorkerPayload::Loaded => return true,
            };
        }
        true
    }
    pub fn track_request(&mut self, purpose: RequestPurpose, request_id: RequestId) {
        if let Some(old) = self.latest_by_purpose.insert(purpose.clone(), request_id) {
            self.purpose_by_request.remove(&old);
        }
        self.purpose_by_request.insert(request_id, purpose);
    }
    pub fn complete_request(&mut self, request_id: RequestId) {
        if let Some(purpose) = self.purpose_by_request.remove(&request_id)
            && self.latest_by_purpose.get(&purpose) == Some(&request_id)
        {
            self.latest_by_purpose.remove(&purpose);
        }
    }
}

#[cfg(test)]
mod notification_tests {
    use super::*;

    #[test]
    fn success_is_visible_and_transient_while_errors_are_actionable_and_persistent() {
        let mut state = AppState::default();
        state
            .notifications
            .push(Notification::success("Saved", "Transaction saved"));
        state
            .notifications
            .push(Notification::actionable_error("Import failed", "Try again"));
        assert!(!state.notifications[0].persistent);
        assert!(state.notifications[1].persistent);
        assert!(state.dismiss_notification(0));
        assert_eq!(state.notifications.len(), 1);
        assert_eq!(state.notifications[0].kind, NotificationKind::Error);
        assert!(state.notifications[0].persistent);
        assert!(state.dismiss_notification(0));
        assert!(state.notifications.is_empty());
    }
}

#[cfg(test)]
mod navigation_tests {
    use super::*;
    use crate::app::navigation::Workspace;

    #[test]
    fn account_and_budget_navigation_preserve_active_budget_and_month() {
        let mut state = AppState::default();
        let budget = BudgetId::new();
        let month = BudgetMonth::new(2026, 8).unwrap();
        state.active_budget = Some(budget);
        state.selected_month = month;
        state.navigation.workspace = Workspace::Account(AccountId::new());
        state.navigation.workspace = Workspace::Budget;
        state.navigation.workspace = Workspace::Overview;
        assert_eq!(state.active_budget, Some(budget));
        assert_eq!(state.selected_month, month);
    }

    #[test]
    fn month_steps_are_budget_local_and_stable_across_transitions() {
        let mut state = AppState::default();
        state.selected_month = BudgetMonth::new(2026, 8).unwrap();
        state.navigation.workspace = Workspace::Reports;
        assert_eq!(state.step_budget_month(true), None);
        assert_eq!(state.selected_month, BudgetMonth::new(2026, 8).unwrap());
        state.navigation.workspace = Workspace::Budget;
        assert_eq!(
            state.step_budget_month(true),
            BudgetMonth::new(2026, 9).ok()
        );
        state.navigation.workspace = Workspace::Overview;
        assert_eq!(state.selected_month, BudgetMonth::new(2026, 9).unwrap());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn projection_refresh_retains_prior_and_rejects_stale_completion() {
        let generation = Generation { budget: 1, view: 2 };
        let mut state = ViewQueryState::default();
        state.begin(1, generation, Some(Id::new("editor")));
        assert!(state.accept(1, generation, "old"));
        state.begin(2, generation, Some(Id::new("editor")));
        assert_eq!(state.last_successful, Some("old"));
        assert!(!state.accept(1, generation, "stale"));
        assert!(state.refresh_active);
        assert!(state.fail(2, generation, "Could not refresh."));
        assert_eq!(state.last_successful, Some("old"));
        assert_eq!(state.preserve_focus, Some(Id::new("editor")));
    }
    #[test]
    fn stale_response_does_not_replace_navigation() {
        let mut s = AppState::default();
        s.track_request(RequestPurpose::Accounts, 2);
        let before = s.navigation;
        assert!(!s.apply_worker_message(WorkerMessage {
            request_id: 1,
            generation: s.generation,
            payload: WorkerPayload::Loaded
        }));
        assert_eq!(s.navigation, before);
    }

    #[test]
    fn worker_response_does_not_steal_search_focus() {
        let context = egui::Context::default();
        let mut state = AppState::default();
        context.memory_mut(|memory| memory.request_focus(state.search_id));
        state.track_request(RequestPurpose::Search, 7);
        assert!(state.apply_worker_message(WorkerMessage {
            request_id: 7,
            generation: state.generation,
            payload: WorkerPayload::Loaded,
        }));
        assert_eq!(
            context.memory(|memory| memory.focused()),
            Some(state.search_id)
        );
    }

    #[test]
    fn unrelated_requests_remain_current() {
        let mut state = AppState::default();
        state.track_request(RequestPurpose::Accounts, 1);
        state.track_request(RequestPurpose::Reports, 2);
        assert!(state.apply_worker_message(WorkerMessage {
            request_id: 1,
            generation: state.generation,
            payload: WorkerPayload::Loaded
        }));
        state.track_request(RequestPurpose::Accounts, 3);
        assert!(!state.apply_worker_message(WorkerMessage {
            request_id: 1,
            generation: state.generation,
            payload: WorkerPayload::Loaded
        }));
        assert!(state.apply_worker_message(WorkerMessage {
            request_id: 2,
            generation: state.generation,
            payload: WorkerPayload::Loaded
        }));
    }
}
