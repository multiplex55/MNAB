use crate::{
    app::{
        message::{WorkerMessage, WorkerPayload},
        navigation::Navigation,
    },
    domain::{AccountId, BudgetId, BudgetMonth, ImportBatchId, Money, TargetId, TransactionId},
    storage::worker::{Generation, RequestId},
};
use egui::Id;
use std::{collections::BTreeMap, path::PathBuf};

#[derive(Clone, Debug)]
pub struct AccountSummary {
    pub id: AccountId,
    pub name: String,
    pub working_balance: Money,
    pub unreconciled: bool,
    pub tracking: bool,
    pub closed: bool,
}
#[derive(Clone, Debug)]
pub enum Dialog {
    ConfirmDelete,
    Reconcile(AccountId),
    Import(AccountId),
    Settings,
}
#[derive(Clone, Debug)]
pub enum InspectorContext {
    Budget,
    Transaction(TransactionId),
    Reconciliation(AccountId),
    Import(ImportBatchId),
    Target(TargetId),
}
#[derive(Clone, Debug)]
pub struct Notification {
    pub kind: NotificationKind,
    pub title: String,
    pub detail: String,
    pub persistent: bool,
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
    pub active_budget: Option<BudgetId>,
    pub budget_name: String,
    pub database_path: Option<PathBuf>,
    pub navigation: Navigation,
    pub selected_account: Option<AccountId>,
    pub selected_month: BudgetMonth,
    pub selected_report: Option<String>,
    pub dialog: Option<DialogState>,
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
    pub search: String,
    pub search_id: Id,
}
impl Default for AppState {
    fn default() -> Self {
        let nav = Navigation::default();
        Self {
            active_budget: None,
            budget_name: "No budget open".into(),
            database_path: None,
            navigation: nav,
            selected_account: None,
            selected_month: nav.month,
            selected_report: None,
            dialog: None,
            notifications: vec![],
            operations: BTreeMap::new(),
            latest_by_purpose: BTreeMap::new(),
            purpose_by_request: BTreeMap::new(),
            generation: Generation { budget: 0, view: 0 },
            inspector_context: InspectorContext::Budget,
            inspector_visible: true,
            sidebar_width: 230.0,
            inspector_width: 280.0,
            accounts: vec![],
            search: String::new(),
            search_id: Id::new("global-search"),
        }
    }
}
impl AppState {
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
mod tests {
    use super::*;
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
