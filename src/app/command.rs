//! Semantic commands emitted by the single global keyboard/router layer.
use crate::domain::{AccountId, BudgetMonth, CategoryId, Money, TransactionId};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AppCommand {
    ContextualNew,
    CreateBudget,
    AddAccount,
    Import,
    FocusSearch,
    Undo,
    Redo,
    Commit,
    Cancel,
    Edit,
    Delete,
    Rename,
    NavigateBudget,
    NavigateReports,
    NavigateAccounts,
    PreviousMonth,
    NextMonth,
    Settings,
    Backup,
    ToggleInspector,
    RetryOperation,
    CancelOperation,
    Exit,
}

/// A persistence-facing command. Payloads deliberately consist only of domain
/// identifiers and owned domain values, keeping storage and UI implementation
/// details on their respective sides of the dispatcher boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FinancialCommand {
    Assign {
        category_id: CategoryId,
        month: BudgetMonth,
        amount: Money,
    },
    DeleteTransaction {
        transaction_id: TransactionId,
        account_id: AccountId,
        month: BudgetMonth,
    },
    AddAccount {
        name: String,
    },
    Import {
        account_id: AccountId,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ApplicationAction {
    Ui(AppCommand),
    Financial(FinancialCommand),
}

pub type CommandId = u64;
pub type CorrelationId = u64;
pub type ConfirmationToken = u64;
pub type FocusRestorationId = u64;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandEnvelope {
    pub command_id: CommandId,
    pub correlation_id: CorrelationId,
    pub budget_generation: u64,
    pub payload: ApplicationAction,
    pub confirmation_token: Option<ConfirmationToken>,
    pub focus_restoration_id: Option<FocusRestorationId>,
}

/// Bounded active-session history. A successful command supplies both redo and inverse payloads;
/// canceled/failed commands simply never call `record_success`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HistoryEntry<C> {
    pub label: String,
    pub command: C,
    pub inverse: C,
}

#[derive(Clone, Debug)]
pub struct CommandHistory<C> {
    undo: std::collections::VecDeque<HistoryEntry<C>>,
    redo: Vec<HistoryEntry<C>>,
    capacity: usize,
}
impl<C: Clone> CommandHistory<C> {
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            undo: std::collections::VecDeque::new(),
            redo: vec![],
            capacity,
        }
    }
    pub fn record_success(&mut self, entry: HistoryEntry<C>) {
        if self.capacity == 0 {
            return;
        }
        self.redo.clear();
        self.undo.push_back(entry);
        while self.undo.len() > self.capacity {
            self.undo.pop_front();
        }
    }
    pub fn undo(&mut self) -> Option<C> {
        let entry = self.undo.pop_back()?;
        let command = entry.inverse.clone();
        self.redo.push(entry);
        Some(command)
    }
    pub fn redo(&mut self) -> Option<C> {
        let entry = self.redo.pop()?;
        let command = entry.command.clone();
        self.undo.push_back(entry);
        Some(command)
    }
    #[must_use]
    pub fn undo_len(&self) -> usize {
        self.undo.len()
    }
    #[must_use]
    pub fn redo_len(&self) -> usize {
        self.redo.len()
    }
    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Reversibility {
    Reversible,
    NonReversible,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OperationNotice {
    pub reversibility: Reversibility,
    pub warning: Option<&'static str>,
}
#[must_use]
pub const fn restore_notice() -> OperationNotice {
    OperationNotice {
        reversibility: Reversibility::NonReversible,
        warning: Some("Restore replaces current data and cannot be undone. Continue?"),
    }
}

#[cfg(test)]
mod history_tests {
    use super::*;
    #[test]
    fn undo_redo_and_branching() {
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
        assert_eq!(h.undo_len(), 1);
        assert_eq!(restore_notice().reversibility, Reversibility::NonReversible);
    }
}
