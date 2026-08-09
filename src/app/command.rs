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
    CreateBudget,
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
    Rename,
    NavigateCategories,
    NavigateReports,
    NavigateAllTransactions,
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
    Update(Account),
    Close(AccountId),
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
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TransactionCommand {
    Save(Transaction),
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
    Budget(BudgetAction),
    Financial(FinancialCommand),
    Category(CategoryAction),
    Report(ReportAction),
}

/// Typed report intents. A retry always carries the original immutable request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReportAction {
    Refresh(crate::domain::ReportRequest),
    Retry(crate::domain::ReportRequest),
    ExportCsv { destination: PathBuf },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BudgetAction {
    ShowCreate,
    ShowOpen,
    ShowRecents,
    Create(crate::service::budget_service::CreateBudget),
    Open(PathBuf),
    Rename {
        budget_id: crate::domain::BudgetId,
        name: String,
    },
    SetArchived {
        budget_id: crate::domain::BudgetId,
        archived: bool,
    },
    RemoveRecent(crate::domain::BudgetId),
    Delete {
        budget_id: crate::domain::BudgetId,
        exact_name: String,
    },
    Reveal(crate::domain::BudgetId),
    Validate(crate::domain::BudgetId),
    Repair {
        budget_id: crate::domain::BudgetId,
        request: crate::storage::repair::RepairRequest,
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
    Categories,
    Reports,
    AllTransactions,
    Inbox,
    AccountRegister,
}

/// Snapshot of UI and lifecycle state needed to evaluate semantic commands.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
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
    pub register_focused: bool,
    pub import_active: bool,
    pub reconciliation_active: bool,
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
    AppCommand::CreateBudget,
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
    AppCommand::ToggleSelection,
    AppCommand::Rename,
    AppCommand::NavigateCategories,
    AppCommand::NavigateReports,
    AppCommand::NavigateAllTransactions,
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
            | NavigateReports
            | NavigateAllTransactions
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
            ) =>
        {
            CommandAvailability::disabled(command, "Open a searchable workspace first")
        }
        Commit if !ctx.editing => {
            CommandAvailability::disabled(command, "Start editing before committing")
        }
        EditAccount | CloseAccount | ReconcileAccount if !ctx.selected_account => {
            CommandAvailability::disabled(command, "Select an active account first")
        }
        EditTransaction | DeleteTransaction if !ctx.selected_transaction => {
            CommandAvailability::disabled(command, "Select a transaction first")
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
        PreviousMonth | NextMonth => CommandAvailability::disabled(
            command,
            "Month navigation is available inside report date filters",
        ),
        CancelOperation if !ctx.mutation_locked && !ctx.lifecycle_busy => {
            CommandAvailability::disabled(command, "No cancellable operation is running")
        }
        RetryOperation if !ctx.mutation_locked => {
            CommandAvailability::disabled(command, "No failed operation is selected")
        }
        _ => CommandAvailability::enabled(command),
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
