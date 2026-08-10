//! Semantic application commands and their runtime-owned execution record.
use std::collections::BTreeSet;
use std::collections::VecDeque;
use std::path::PathBuf;

use crate::{
    app::inbox::InboxItemId,
    domain::{
        Account, AccountId, BudgetAssignment, BudgetMonth, Category, CategoryId, ImportBatchId,
        Payee, PayeeId, ReconciliationId, ScheduledTransactionId, Target, TargetId, Transaction,
        TransactionId,
    },
    storage::worker::{RequestId, SafeUserError},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AppCommand {
    ContextualNew,
    CompleteOnboarding,
    AddAccount,
    EditAccount,
    CloseAccount,
    AddAccountGroup,
    RenameAccountGroup,
    DeleteAccountGroup,
    MoveAccountGroup,
    AddTransaction,
    EditTransaction,
    DeleteTransaction,
    CreateTransfer,
    ReconcileAccount,
    Import,
    FocusSearch,
    Undo,
    Redo,
    Commit,
    Cancel,
    Edit,
    Delete,
    MoveUp,
    MoveDown,
    NextField,
    PreviousField,
    ToggleSelection,
    SelectAllTransactions,
    ResetRegisterColumns,
    PersistRegisterView,
    Rename,
    NavigateOverview,
    NavigateBudget,
    NavigateCategories,
    NavigateReports,
    NavigateAllTransactions,
    AutoAssign,
    MoveMoney,
    PreviousMonth,
    NextMonth,
    Settings,
    Backup,
    ToggleInspector,
    RetryOperation,
    CancelOperation,
    Exit,
}

/// Persistence commands are grouped by aggregate. Payloads contain domain identities and values,
/// never widget positions, paths, SQL, or UI-framework state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FinancialCommand {
    Account(AccountCommand),
    Category(CategoryCommand),
    Assignment(AssignmentCommand),
    Transaction(TransactionCommand),
    Payee(PayeeCommand),
    Import(ImportCommand),
    Reconciliation(ReconciliationCommand),
    Target(TargetCommand),
    Schedule(ScheduleCommand),
    Inbox(InboxCommand),
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AccountCommand {
    Create(Account),
    CreateWithOpening {
        account: Account,
        opening_magnitude: crate::domain::Money,
        opening_date: crate::domain::TransactionDate,
    },
    Update(Account),
    Close(AccountId),
    Reopen(AccountId),
    SetFavorite {
        id: AccountId,
        favorite: bool,
    },
    MoveToGroup {
        id: AccountId,
        group_id: Option<crate::domain::AccountGroupId>,
    },
    DeleteUnused(AccountId),
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CategoryCommand {
    Update(Category),
    Delete(CategoryId),
    CreateGroup(crate::domain::CategoryGroup),
    ReorderGroup {
        id: crate::domain::CategoryGroupId,
        before: Option<crate::domain::CategoryGroupId>,
    },
    ReorderCategory {
        id: CategoryId,
        group_id: crate::domain::CategoryGroupId,
        before: Option<CategoryId>,
    },
    Merge {
        source: CategoryId,
        destination: CategoryId,
    },
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AssignmentCommand {
    Set(BudgetAssignment),
    Remove {
        category_id: CategoryId,
        month: BudgetMonth,
    },
    /// An all-or-nothing assignment edit based on a particular budget snapshot.
    Batch(AssignmentBatch),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssignmentBatch {
    pub month: BudgetMonth,
    pub expected_source_revision: u64,
    pub changes: Vec<AssignmentBatchChange>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AssignmentBatchChange {
    Set {
        category_id: CategoryId,
        amount: crate::domain::Money,
    },
    Remove {
        category_id: CategoryId,
    },
}

impl AssignmentBatchChange {
    #[must_use]
    pub const fn category_id(&self) -> CategoryId {
        match self {
            Self::Set { category_id, .. } | Self::Remove { category_id } => *category_id,
        }
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TransactionCommand {
    Save(Transaction),
    SaveTransfer {
        source: Transaction,
        destination: Transaction,
    },
    Delete {
        transaction_id: TransactionId,
        account_id: AccountId,
        month: BudgetMonth,
    },
    /// A query-wide transaction mutation. `AllMatching` is resolved by the worker, inside the
    /// write transaction; the UI never expands it into row commands.
    Batch(TransactionBatchCommand),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TransactionBatchSelection {
    Explicit(BTreeSet<TransactionId>),
    AllMatching {
        query: crate::app::register::CanonicalQuery,
        exclusions: BTreeSet<TransactionId>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TransactionBatchAction {
    SetApproval(crate::domain::Approval),
    SetCategory(CategoryId),
    SetPayee(Option<PayeeId>),
    SetClearance(crate::domain::Clearance),
    SetMemo(Option<String>),
    Void,
    Delete,
    /// Lossless, typed inverse used by undo; not exposed as an ordinary UI action.
    Restore(Vec<Transaction>),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransactionBatchCommand {
    pub selection: TransactionBatchSelection,
    pub action: TransactionBatchAction,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PayeeCommand {
    Update(Payee),
    Delete(PayeeId),
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ImportCommand {
    Apply {
        batch_id: ImportBatchId,
        account_id: AccountId,
    },
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReconciliationCommand {
    Complete(ReconciliationId),
    CompleteSnapshot(crate::domain::Reconciliation),
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TargetCommand {
    Save(Target),
    Delete(TargetId),
}

/// Presentation intents which need routing but are not themselves persistence commands.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CategoryAction {
    RefreshCatalog,
    Select(CategoryId),
    ToggleArchived(bool),
    NewGroup,
    NewCategory(crate::domain::CategoryGroupId),
    Edit(CategoryId),
    OpenActivity(CategoryId),
    OpenTransactions(CategoryId),
    BeginGoal(CategoryId),
    BeginGoalTransfer(CategoryId),
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ScheduleCommand {
    Delete(ScheduledTransactionId),
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InboxCommand {
    Resolve {
        item_id: InboxItemId,
        action: InboxAction,
    },
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum InboxAction {
    Approve,
    Categorize,
    Match,
    EnterOccurrence,
    SkipOccurrence,
    Clear,
    Reconcile,
    MoveMoney,
    OpenTarget,
    ViewFailure,
    Dismiss,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ApplicationAction {
    RequestExit,
    Ui(AppCommand),
    /// Non-financial budget lifecycle intent. These actions are deliberately
    /// distinct from worker commands and never enter undo/redo history.
    Data(DataAction),
    Financial(FinancialCommand),
    Category(CategoryAction),
    Report(ReportAction),
    Register(RegisterAction),
}

/// Register gestures carry stable identities; an index is never allowed to escape the widget.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RegisterAction {
    Click {
        id: TransactionId,
        ctrl: bool,
        shift: bool,
    },
    Move {
        delta: isize,
        extend: bool,
    },
    ToggleCurrent,
    BeginEdit(TransactionId),
    SetClearance {
        id: TransactionId,
        clearance: crate::domain::Clearance,
    },
    Approve(TransactionId),
    Delete(TransactionId),
}

/// Typed report intents. A retry always carries the original immutable request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReportAction {
    Refresh(crate::domain::ReportRequest),
    Retry(crate::domain::ReportRequest),
    ExportCsv { destination: PathBuf },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DataAction {
    CreateBackup,
    RestoreBackup {
        metadata_path: PathBuf,
        confirmed: bool,
    },
    Validate,
    Repair {
        request: crate::storage::repair::RepairRequest,
        confirmed: bool,
    },
    RevealDataDirectory,
    RevealBackupDirectory,
    RenameBudget {
        name: String,
    },
}

pub type CommandId = u64;
pub type CorrelationId = u64;
pub type ConfirmationToken = u64;
pub type FocusRestorationId = u64;
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct DeduplicationKey(pub String);
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandEnvelope {
    pub command_id: CommandId,
    pub correlation_id: CorrelationId,
    pub budget_generation: u64,
    pub payload: ApplicationAction,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandStatus {
    Queued,
    AwaitingConfirmation,
    Submitting,
    Running,
    Committed,
    Failed,
    Cancelled,
}
impl CommandStatus {
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Committed | Self::Failed | Self::Cancelled)
    }
    #[must_use]
    pub const fn can_transition_to(self, to: Self) -> bool {
        matches!(
            (self, to),
            (
                Self::Queued,
                Self::AwaitingConfirmation | Self::Submitting | Self::Cancelled | Self::Failed
            ) | (
                Self::AwaitingConfirmation,
                Self::Submitting | Self::Cancelled | Self::Failed
            ) | (
                Self::Submitting,
                Self::Running | Self::Failed | Self::Cancelled
            ) | (
                Self::Running,
                Self::Committed | Self::Failed | Self::Cancelled
            ) | (Self::Failed, Self::Submitting)
        )
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfirmationState {
    NotRequired,
    Required,
    Confirmed(ConfirmationToken),
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Reversibility {
    Reversible,
    NonReversible,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CancellationPolicy {
    Cancellable,
    MustComplete,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperationClass {
    Mutation,
    Import,
    Reconciliation,
    Maintenance,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FailureSafety {
    Retryable(SafeUserError),
    NonRetryable(SafeUserError),
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RetryMetadata {
    pub attempts: u32,
    pub max_attempts: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeCommand {
    pub envelope: CommandEnvelope,
    pub status: CommandStatus,
    pub worker_request_id: Option<RequestId>,
    pub confirmation: ConfirmationState,
    pub focus_restoration_id: Option<FocusRestorationId>,
    pub operation_label: String,
    pub reversibility: Reversibility,
    pub cancellation_policy: CancellationPolicy,
    pub retry: RetryMetadata,
    pub operation_class: OperationClass,
    pub safe_failure: Option<FailureSafety>,
    pub deduplication_key: DeduplicationKey,
    pub terminal_sequence: Option<u64>,
}
impl RuntimeCommand {
    pub fn transition(&mut self, to: CommandStatus) -> Result<(), TransitionError> {
        if !self.status.can_transition_to(to) {
            return Err(TransitionError {
                from: self.status,
                to,
            });
        }
        self.status = to;
        Ok(())
    }
    #[must_use]
    pub fn response_matches(
        &self,
        request_id: RequestId,
        command_id: CommandId,
        correlation_id: CorrelationId,
        budget_generation: u64,
    ) -> bool {
        self.worker_request_id == Some(request_id)
            && self.envelope.command_id == command_id
            && self.envelope.correlation_id == correlation_id
            && self.envelope.budget_generation == budget_generation
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TransitionError {
    pub from: CommandStatus,
    pub to: CommandStatus,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HistoryEntry<C> {
    pub label: String,
    pub command: C,
    pub inverse: C,
}
#[derive(Clone, Debug)]
pub struct CommandHistory<C> {
    undo: VecDeque<HistoryEntry<C>>,
    redo: Vec<HistoryEntry<C>>,
    capacity: usize,
}
impl<C: Clone> CommandHistory<C> {
    pub fn new(capacity: usize) -> Self {
        Self {
            undo: VecDeque::new(),
            redo: vec![],
            capacity,
        }
    }
    pub fn record_success(&mut self, e: HistoryEntry<C>) {
        if self.capacity == 0 {
            return;
        }
        self.redo.clear();
        self.undo.push_back(e);
        while self.undo.len() > self.capacity {
            self.undo.pop_front();
        }
    }
    pub fn undo(&mut self) -> Option<C> {
        let e = self.undo.pop_back()?;
        let c = e.inverse.clone();
        self.redo.push(e);
        Some(c)
    }
    pub fn next_undo(&self) -> Option<C> {
        self.undo.back().map(|e| e.inverse.clone())
    }
    pub fn next_redo(&self) -> Option<C> {
        self.redo.last().map(|e| e.command.clone())
    }
    pub fn redo(&mut self) -> Option<C> {
        let e = self.redo.pop()?;
        let c = e.command.clone();
        self.undo.push_back(e);
        Some(c)
    }
    /// Rebinds the command contract after an undo has committed. This is important for
    /// revision-checked commands: redo must use the revision produced by the undo transaction.
    pub fn replace_next_redo(&mut self, command: C) {
        if let Some(entry) = self.redo.last_mut() {
            entry.command = command;
        }
    }
    /// Rebinds the inverse contract after a redo has committed.
    pub fn replace_next_undo(&mut self, inverse: C) {
        if let Some(entry) = self.undo.back_mut() {
            entry.inverse = inverse;
        }
    }
    pub fn undo_len(&self) -> usize {
        self.undo.len()
    }
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
    pub fn undo_label(&self) -> Option<&str> {
        self.undo.back().map(|e| e.label.as_str())
    }
    pub fn redo_label(&self) -> Option<&str> {
        self.redo.last().map(|e| e.label.as_str())
    }
    pub fn redo_len(&self) -> usize {
        self.redo.len()
    }
    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OperationNotice {
    pub reversibility: Reversibility,
    pub warning: Option<&'static str>,
}
pub const fn restore_notice() -> OperationNotice {
    OperationNotice {
        reversibility: Reversibility::NonReversible,
        warning: Some("Restore replaces current data and cannot be undone. Continue?"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_transition_is_centrally_classified() {
        let all = [
            CommandStatus::Queued,
            CommandStatus::AwaitingConfirmation,
            CommandStatus::Submitting,
            CommandStatus::Running,
            CommandStatus::Committed,
            CommandStatus::Failed,
            CommandStatus::Cancelled,
        ];
        let legal = [
            (CommandStatus::Queued, CommandStatus::AwaitingConfirmation),
            (CommandStatus::Queued, CommandStatus::Submitting),
            (CommandStatus::Queued, CommandStatus::Cancelled),
            (CommandStatus::Queued, CommandStatus::Failed),
            (
                CommandStatus::AwaitingConfirmation,
                CommandStatus::Submitting,
            ),
            (
                CommandStatus::AwaitingConfirmation,
                CommandStatus::Cancelled,
            ),
            (CommandStatus::AwaitingConfirmation, CommandStatus::Failed),
            (CommandStatus::Submitting, CommandStatus::Running),
            (CommandStatus::Submitting, CommandStatus::Failed),
            (CommandStatus::Submitting, CommandStatus::Cancelled),
            (CommandStatus::Running, CommandStatus::Committed),
            (CommandStatus::Running, CommandStatus::Failed),
            (CommandStatus::Running, CommandStatus::Cancelled),
            (CommandStatus::Failed, CommandStatus::Submitting),
        ];
        for from in all {
            for to in all {
                assert_eq!(
                    from.can_transition_to(to),
                    legal.contains(&(from, to)),
                    "{from:?}->{to:?}"
                );
            }
        }
    }
    #[test]
    fn history_branches_only_on_success() {
        let mut h = CommandHistory::new(2);
        h.record_success(HistoryEntry {
            label: "one".into(),
            command: 1,
            inverse: -1,
        });
        assert_eq!(h.undo(), Some(-1));
        assert_eq!(h.redo(), Some(1));
        assert_eq!(h.undo(), Some(-1));
        h.record_success(HistoryEntry {
            label: "two".into(),
            command: 2,
            inverse: -2,
        });
        assert_eq!(h.redo_len(), 0);
    }
}

/// High-level workspace used by the centralized command availability evaluator.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum CommandWorkspace {
    #[default]
    None,
    Overview,
    Budget,
    Categories,
    Reports,
    AllTransactions,
    Inbox,
    AccountRegister,
}

/// Snapshot of UI and lifecycle state needed to evaluate semantic commands.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CommandAvailabilityContext {
    pub database_available: bool,
    pub workspace: CommandWorkspace,
    pub has_selection: bool,
    pub editing: bool,
    pub dialog_open: bool,
    pub text_editor_owns_shortcuts: bool,
    pub lifecycle_busy: bool,
    pub read_only: bool,
    pub mutation_locked: bool,
    pub can_undo: bool,
    pub can_redo: bool,
    pub selected_account: bool,
    pub selected_transaction: bool,
    /// The current register selection contains at least one protected reconciled row.
    pub selected_reconciled_transaction: bool,
    pub register_focused: bool,
    pub import_active: bool,
    pub reconciliation_active: bool,
    /// At least one category is selected in the budget grid.
    pub budget_selection: bool,
    /// The selected categories produce at least one Auto-Assign change.
    pub auto_assign_selection: bool,
    /// The active editor has passed its synchronous validation.
    pub editor_valid: bool,
    /// A storage worker can accept requests.
    pub worker_available: bool,
}

impl Default for CommandAvailabilityContext {
    fn default() -> Self {
        Self {
            database_available: false,
            workspace: CommandWorkspace::None,
            has_selection: false,
            editing: false,
            dialog_open: false,
            text_editor_owns_shortcuts: false,
            lifecycle_busy: false,
            read_only: false,
            mutation_locked: false,
            can_undo: false,
            can_redo: false,
            selected_account: false,
            selected_transaction: false,
            selected_reconciled_transaction: false,
            register_focused: false,
            import_active: false,
            reconciliation_active: false,
            budget_selection: false,
            auto_assign_selection: false,
            editor_valid: true,
            worker_available: true,
        }
    }
}

/// Canonical presentation and execution contract consumed by every action surface.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActionDescriptor {
    pub action: AppCommand,
    pub title: &'static str,
    pub shortcut: Option<&'static str>,
    pub visible: bool,
    pub enabled: bool,
    pub disabled_reason: Option<&'static str>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandAvailability {
    pub command: AppCommand,
    pub enabled: bool,
    pub disabled_reason: Option<&'static str>,
    pub exposed_in_palette: bool,
}

impl CommandAvailability {
    const fn enabled(command: AppCommand) -> Self {
        Self {
            command,
            enabled: true,
            disabled_reason: None,
            exposed_in_palette: true,
        }
    }
    const fn disabled(command: AppCommand, reason: &'static str) -> Self {
        Self {
            command,
            enabled: false,
            disabled_reason: Some(reason),
            exposed_in_palette: true,
        }
    }
}

pub const MAJOR_WORKFLOW_COMMANDS: &[AppCommand] = &[
    AppCommand::ContextualNew,
    AppCommand::CompleteOnboarding,
    AppCommand::AddAccount,
    AppCommand::EditAccount,
    AppCommand::CloseAccount,
    AppCommand::AddAccountGroup,
    AppCommand::RenameAccountGroup,
    AppCommand::DeleteAccountGroup,
    AppCommand::MoveAccountGroup,
    AppCommand::AddTransaction,
    AppCommand::EditTransaction,
    AppCommand::DeleteTransaction,
    AppCommand::CreateTransfer,
    AppCommand::ReconcileAccount,
    AppCommand::Import,
    AppCommand::FocusSearch,
    AppCommand::Undo,
    AppCommand::Redo,
    AppCommand::Commit,
    AppCommand::Cancel,
    AppCommand::Edit,
    AppCommand::Delete,
    AppCommand::MoveUp,
    AppCommand::MoveDown,
    AppCommand::NextField,
    AppCommand::PreviousField,
    AppCommand::ToggleSelection,
    AppCommand::SelectAllTransactions,
    AppCommand::ResetRegisterColumns,
    AppCommand::PersistRegisterView,
    AppCommand::Rename,
    AppCommand::NavigateOverview,
    AppCommand::NavigateBudget,
    AppCommand::NavigateCategories,
    AppCommand::NavigateReports,
    AppCommand::NavigateAllTransactions,
    AppCommand::AutoAssign,
    AppCommand::MoveMoney,
    AppCommand::PreviousMonth,
    AppCommand::NextMonth,
    AppCommand::Settings,
    AppCommand::Backup,
    AppCommand::ToggleInspector,
    AppCommand::RetryOperation,
    AppCommand::CancelOperation,
    AppCommand::Exit,
];

#[must_use]
pub fn command_availability(
    ctx: CommandAvailabilityContext,
    command: AppCommand,
) -> CommandAvailability {
    use AppCommand::*;
    if ctx.text_editor_owns_shortcuts
        && matches!(
            command,
            ContextualNew
                | FocusSearch
                | Import
                | AddAccount
                | EditAccount
                | CloseAccount
                | AddAccountGroup
                | RenameAccountGroup
                | DeleteAccountGroup
                | MoveAccountGroup
                | AddTransaction
                | EditTransaction
                | DeleteTransaction
                | CreateTransfer
                | ReconcileAccount
                | Delete
                | Rename
                | MoveUp
                | MoveDown
                | ToggleSelection
                | PreviousMonth
                | NextMonth
        )
    {
        return CommandAvailability::disabled(command, "Text editing has focus");
    }
    if ctx.dialog_open {
        return match command {
            Commit | Cancel => CommandAvailability::enabled(command),
            _ => CommandAvailability::disabled(command, "Finish the open dialog first"),
        };
    }
    if ctx.lifecycle_busy && !matches!(command, CancelOperation | RetryOperation | Exit) {
        return CommandAvailability::disabled(command, "A budget lifecycle operation is running");
    }
    let needs_budget = matches!(
        command,
        ContextualNew
            | AddAccount
            | EditAccount
            | CloseAccount
            | AddAccountGroup
            | RenameAccountGroup
            | DeleteAccountGroup
            | MoveAccountGroup
            | AddTransaction
            | EditTransaction
            | DeleteTransaction
            | CreateTransfer
            | ReconcileAccount
            | Import
            | FocusSearch
            | Undo
            | Redo
            | Commit
            | Edit
            | Delete
            | ToggleSelection
            | Rename
            | NavigateCategories
            | NavigateOverview
            | NavigateBudget
            | NavigateReports
            | NavigateAllTransactions
            | AutoAssign
            | MoveMoney
            | PreviousMonth
            | NextMonth
            | Backup
    );
    if needs_budget && !ctx.database_available {
        return CommandAvailability::disabled(command, "Open a budget first");
    }
    let mutating = matches!(
        command,
        ContextualNew
            | AddAccount
            | EditAccount
            | CloseAccount
            | AddAccountGroup
            | RenameAccountGroup
            | DeleteAccountGroup
            | MoveAccountGroup
            | AddTransaction
            | EditTransaction
            | DeleteTransaction
            | CreateTransfer
            | ReconcileAccount
            | Import
            | Commit
            | Delete
            | Rename
            | Undo
            | Redo
    );
    if mutating && ctx.read_only {
        return CommandAvailability::disabled(command, "Budget is open read-only");
    }
    if mutating && ctx.mutation_locked {
        return CommandAvailability::disabled(command, "Another operation must finish first");
    }
    if mutating && !ctx.worker_available {
        return CommandAvailability::disabled(command, "The storage worker is unavailable");
    }
    match command {
        ContextualNew | Import if ctx.workspace != CommandWorkspace::AccountRegister => {
            CommandAvailability::disabled(command, "Open an account register first")
        }
        FocusSearch
            if !matches!(
                ctx.workspace,
                CommandWorkspace::AccountRegister
                    | CommandWorkspace::AllTransactions
                    | CommandWorkspace::Inbox
                    | CommandWorkspace::Reports
            ) =>
        {
            CommandAvailability::disabled(command, "Open a searchable workspace first")
        }
        Commit if !ctx.editing => {
            CommandAvailability::disabled(command, "Start editing before committing")
        }
        Commit if !ctx.editor_valid => CommandAvailability::disabled(
            command,
            "Resolve the editor validation errors before saving",
        ),
        EditAccount | CloseAccount | ReconcileAccount if !ctx.selected_account => {
            CommandAvailability::disabled(command, "Select an active account first")
        }
        AddTransaction
            if !matches!(
                ctx.workspace,
                CommandWorkspace::AccountRegister | CommandWorkspace::AllTransactions
            ) =>
        {
            CommandAvailability::disabled(command, "Open a transaction register first")
        }
        EditTransaction | DeleteTransaction if !ctx.selected_transaction => {
            CommandAvailability::disabled(command, "Select a transaction first")
        }
        Delete | DeleteTransaction if ctx.selected_reconciled_transaction => {
            CommandAvailability::disabled(
                command,
                "Reconciled transactions cannot be changed in bulk; deselect them or edit them individually.",
            )
        }
        RenameAccountGroup | DeleteAccountGroup | MoveAccountGroup => {
            CommandAvailability::disabled(command, "Select an account group first")
        }
        Edit if ctx.editing => CommandAvailability::disabled(command, "Already editing"),
        Edit | Delete | Rename if !ctx.has_selection => {
            CommandAvailability::disabled(command, "Select an item first")
        }
        ToggleSelection if !ctx.has_selection => {
            CommandAvailability::disabled(command, "Move to an item before changing selection")
        }
        MoveUp | MoveDown if ctx.editing => {
            CommandAvailability::disabled(command, "Finish editing before navigating rows")
        }
        Undo if !ctx.can_undo => CommandAvailability::disabled(command, "Nothing to undo"),
        Redo if !ctx.can_redo => CommandAvailability::disabled(command, "Nothing to redo"),
        PreviousMonth | NextMonth if ctx.workspace != CommandWorkspace::Budget => {
            CommandAvailability::disabled(command, "Open Budget to change its month")
        }
        AutoAssign | MoveMoney if ctx.workspace != CommandWorkspace::Budget => {
            CommandAvailability::disabled(command, "Open Budget to plan money")
        }
        AutoAssign if !ctx.budget_selection => {
            CommandAvailability::disabled(command, "Select at least one budget category first")
        }
        AutoAssign if !ctx.auto_assign_selection => CommandAvailability::disabled(
            command,
            "The Auto-Assign selection has no changes to apply",
        ),
        MoveMoney if !ctx.budget_selection => {
            CommandAvailability::disabled(command, "Select at least one budget category first")
        }
        CancelOperation if !ctx.mutation_locked && !ctx.lifecycle_busy => {
            CommandAvailability::disabled(command, "No cancellable operation is running")
        }
        RetryOperation if !ctx.mutation_locked => {
            CommandAvailability::disabled(command, "No failed operation is selected")
        }
        _ => CommandAvailability::enabled(command),
    }
}

/// Builds the single descriptor used by buttons, menus, shortcuts, cards, and the palette.
#[must_use]
pub fn action_descriptor(ctx: CommandAvailabilityContext, action: AppCommand) -> ActionDescriptor {
    use AppCommand::*;
    let title = match action {
        ContextualNew => "New",
        CompleteOnboarding => "Finish setup",
        AddAccount => "New account",
        EditAccount => "Edit account",
        CloseAccount => "Close account",
        AddAccountGroup => "New account group",
        RenameAccountGroup => "Rename account group",
        DeleteAccountGroup => "Delete account group",
        MoveAccountGroup => "Move account group",
        AddTransaction => "New transaction",
        EditTransaction => "Edit transaction",
        DeleteTransaction => "Delete transaction",
        CreateTransfer => "New transfer",
        ReconcileAccount => "Reconcile account",
        Import => "Import transactions",
        FocusSearch => "Find",
        Undo => "Undo",
        Redo => "Redo",
        Commit => "Save Current Edit",
        Cancel => "Cancel",
        Edit => "Edit selected item",
        Delete => "Delete selected item",
        MoveUp => "Move up",
        MoveDown => "Move down",
        NextField => "Next field",
        PreviousField => "Previous field",
        ToggleSelection => "Toggle selection",
        SelectAllTransactions => "Select all transactions",
        ResetRegisterColumns => "Reset register columns",
        PersistRegisterView => "Save register view",
        Rename => "Rename",
        NavigateOverview => "Open overview",
        NavigateBudget => "View budget",
        NavigateCategories => "Manage categories",
        NavigateReports => "Open reports",
        NavigateAllTransactions => "Manage accounts",
        AutoAssign => "Auto-Assign",
        MoveMoney => "Move Money",
        PreviousMonth => "Previous month",
        NextMonth => "Next month",
        Settings => "Open settings",
        Backup => "Create backup",
        ToggleInspector => "Toggle inspector",
        RetryOperation => "Retry operation",
        CancelOperation => "Cancel operation",
        Exit => "Exit",
    };
    let availability = command_availability(ctx, action);
    ActionDescriptor {
        action,
        title,
        shortcut: crate::app::palette::shortcut(action),
        visible: true,
        enabled: availability.enabled,
        disabled_reason: availability.disabled_reason,
    }
}

#[must_use]
pub fn command_catalog(ctx: CommandAvailabilityContext) -> Vec<CommandAvailability> {
    MAJOR_WORKFLOW_COMMANDS
        .iter()
        .copied()
        .map(|c| command_availability(ctx, c))
        .collect()
}

#[cfg(test)]
mod availability_tests {
    use super::*;

    #[test]
    fn reconciled_register_selection_explains_disabled_bulk_delete() {
        let context = CommandAvailabilityContext {
            database_available: true,
            workspace: CommandWorkspace::AccountRegister,
            has_selection: true,
            selected_transaction: true,
            selected_reconciled_transaction: true,
            ..Default::default()
        };
        let availability = command_availability(context, AppCommand::DeleteTransaction);
        assert!(!availability.enabled);
        assert_eq!(
            availability.disabled_reason,
            Some(
                "Reconciled transactions cannot be changed in bulk; deselect them or edit them individually."
            )
        );
    }

    #[test]
    fn every_visible_enabled_application_action_has_a_runtime_contract() {
        let context = CommandAvailabilityContext {
            database_available: true,
            workspace: CommandWorkspace::AccountRegister,
            selected_account: true,
            selected_transaction: true,
            has_selection: true,
            can_undo: true,
            can_redo: true,
            budget_selection: true,
            auto_assign_selection: true,
            ..Default::default()
        };
        for action in MAJOR_WORKFLOW_COMMANDS {
            let descriptor = action_descriptor(context, *action);
            assert_eq!(descriptor.action, *action);
            assert!(!descriptor.title.is_empty());
            assert!(descriptor.visible);
            if descriptor.enabled {
                assert!(descriptor.disabled_reason.is_none(), "{action:?}");
            }
        }
    }

    #[test]
    fn required_disabled_reasons_are_explicit() {
        let no_worker = command_availability(
            CommandAvailabilityContext {
                database_available: true,
                worker_available: false,
                ..Default::default()
            },
            AppCommand::AddAccount,
        );
        assert_eq!(
            no_worker.disabled_reason,
            Some("The storage worker is unavailable")
        );
        let no_budget_selection = command_availability(
            CommandAvailabilityContext {
                database_available: true,
                workspace: CommandWorkspace::Budget,
                ..Default::default()
            },
            AppCommand::AutoAssign,
        );
        assert_eq!(
            no_budget_selection.disabled_reason,
            Some("Select at least one budget category first")
        );
    }
}
