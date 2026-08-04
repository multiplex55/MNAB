//! UI-independent application lifetime policy.
//!
//! This module deliberately knows nothing about eframe/egui.  The application
//! adapter is responsible for translating [`LifecycleEffect`] values into native
//! window commands.
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum LifecycleState {
    #[default]
    Running,
    ShutdownRequested,
    ReviewingShutdown,
    ShuttingDown,
    ShutdownFailed,
    ReadyToClose,
    Exiting,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LifecycleEffect {
    CancelNativeClose,
    BeginDraftReview,
    BeginShutdown,
    ShowShutdownFailure,
    SendProgrammaticClose,
    AllowClose,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IllegalTransition {
    pub from: LifecycleState,
    pub to: LifecycleState,
}

#[derive(Debug, Default)]
pub struct Lifecycle {
    state: LifecycleState,
    programmatic_close_sent: bool,
}

impl Lifecycle {
    pub const fn state(&self) -> LifecycleState {
        self.state
    }

    pub fn transition(&mut self, to: LifecycleState) -> Result<(), IllegalTransition> {
        use LifecycleState as S;
        let legal = matches!(
            (self.state, to),
            (S::Running, S::ShutdownRequested)
                | (S::ShutdownRequested, S::ReviewingShutdown)
                | (S::ReviewingShutdown, S::Running | S::ShuttingDown)
                | (S::ShuttingDown, S::ShutdownFailed | S::ReadyToClose)
                | (S::ShutdownFailed, S::ShuttingDown | S::Exiting)
                | (S::ReadyToClose, S::Exiting)
        );
        if !legal {
            return Err(IllegalTransition {
                from: self.state,
                to,
            });
        }
        self.state = to;
        Ok(())
    }

    /// Handles both the title-bar close button and Alt+F4.
    pub fn native_close_requested(&mut self) -> Vec<LifecycleEffect> {
        use LifecycleEffect as E;
        use LifecycleState as S;
        match self.state {
            S::Running => {
                self.transition(S::ShutdownRequested)
                    .expect("legal close transition");
                vec![E::CancelNativeClose, E::BeginDraftReview]
            }
            S::ShutdownRequested | S::ReviewingShutdown | S::ShuttingDown | S::ShutdownFailed => {
                vec![E::CancelNativeClose]
            }
            S::ReadyToClose => {
                self.transition(S::Exiting).expect("legal close transition");
                vec![E::AllowClose]
            }
            S::Exiting => vec![E::AllowClose],
        }
    }

    pub fn request_exit(&mut self) -> Vec<LifecycleEffect> {
        if self.state != LifecycleState::Running {
            return Vec::new();
        }
        self.transition(LifecycleState::ShutdownRequested)
            .expect("legal exit transition");
        vec![LifecycleEffect::BeginDraftReview]
    }

    pub fn begin_review(&mut self) -> Result<(), IllegalTransition> {
        self.transition(LifecycleState::ReviewingShutdown)
    }
    pub fn begin_shutdown(&mut self) -> Result<LifecycleEffect, IllegalTransition> {
        self.transition(LifecycleState::ShuttingDown)?;
        Ok(LifecycleEffect::BeginShutdown)
    }
    pub fn shutdown_failed(&mut self) -> Result<LifecycleEffect, IllegalTransition> {
        self.transition(LifecycleState::ShutdownFailed)?;
        Ok(LifecycleEffect::ShowShutdownFailure)
    }
    pub fn shutdown_succeeded(&mut self) -> Result<Vec<LifecycleEffect>, IllegalTransition> {
        if self.state == LifecycleState::ReadyToClose && self.programmatic_close_sent {
            return Ok(Vec::new());
        }
        self.transition(LifecycleState::ReadyToClose)?;
        if self.programmatic_close_sent {
            return Ok(Vec::new());
        }
        self.programmatic_close_sent = true;
        Ok(vec![LifecycleEffect::SendProgrammaticClose])
    }
    pub fn retry_shutdown(&mut self) -> Result<LifecycleEffect, IllegalTransition> {
        self.begin_shutdown()
    }
    pub fn force_exit(&mut self) -> Result<LifecycleEffect, IllegalTransition> {
        self.transition(LifecycleState::Exiting)?;
        Ok(LifecycleEffect::SendProgrammaticClose)
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct EditorId(pub u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DraftStatus {
    Clean,
    ModifiedAndValid,
    ModifiedAndInvalid,
}

pub trait Draft {
    fn draft_status(&self) -> DraftStatus;
    fn commit_draft(&mut self) -> Result<(), String>;
    fn discard_draft(&mut self);
    fn focus_invalid_field(&mut self);
}

#[derive(Default)]
pub struct DraftRegistry {
    drafts: BTreeMap<EditorId, Box<dyn Draft>>,
}
impl DraftRegistry {
    pub fn register(&mut self, id: EditorId, draft: Box<dyn Draft>) {
        self.drafts.insert(id, draft);
    }
    pub fn unregister(&mut self, id: EditorId) {
        self.drafts.remove(&id);
    }
    pub fn review(&mut self) -> Result<Option<EditorId>, String> {
        for (id, draft) in &mut self.drafts {
            match draft.draft_status() {
                DraftStatus::Clean => draft.discard_draft(),
                DraftStatus::ModifiedAndValid => draft.commit_draft()?,
                DraftStatus::ModifiedAndInvalid => return Ok(Some(*id)),
            }
        }
        Ok(None)
    }
    pub fn return_to_editor(&mut self, id: EditorId) {
        if let Some(draft) = self.drafts.get_mut(&id) {
            draft.focus_invalid_field();
        }
    }
    pub fn discard(&mut self, id: EditorId) {
        if let Some(draft) = self.drafts.get_mut(&id) {
            draft.discard_draft();
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperationClass {
    CancellableRead,
    FinancialMutation,
    ReviewSensitive,
    CriticalExclusive,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperationShutdownAction {
    Cancel,
    Wait,
    Confirm,
    Block,
}

#[derive(Default)]
pub struct OperationRegistry(BTreeMap<u64, OperationClass>);
impl OperationRegistry {
    pub fn register(&mut self, id: u64, class: OperationClass) {
        self.0.insert(id, class);
    }
    pub fn complete(&mut self, id: u64) {
        self.0.remove(&id);
    }
    pub fn shutdown_actions(&self) -> Vec<(u64, OperationShutdownAction)> {
        self.0
            .iter()
            .map(|(&id, class)| {
                (
                    id,
                    match class {
                        OperationClass::CancellableRead => OperationShutdownAction::Cancel,
                        OperationClass::FinancialMutation => OperationShutdownAction::Wait,
                        OperationClass::ReviewSensitive => OperationShutdownAction::Confirm,
                        OperationClass::CriticalExclusive => OperationShutdownAction::Block,
                    },
                )
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    struct FakeDraft {
        status: DraftStatus,
        events: Arc<Mutex<Vec<&'static str>>>,
        fail: bool,
    }
    impl Draft for FakeDraft {
        fn draft_status(&self) -> DraftStatus {
            self.status
        }
        fn commit_draft(&mut self) -> Result<(), String> {
            self.events.lock().unwrap().push("commit");
            if self.fail {
                Err("commit failed".into())
            } else {
                Ok(())
            }
        }
        fn discard_draft(&mut self) {
            self.events.lock().unwrap().push("discard");
        }
        fn focus_invalid_field(&mut self) {
            self.events.lock().unwrap().push("focus");
        }
    }
    #[test]
    fn native_close_is_cancelled_until_one_programmatic_close() {
        let mut life = Lifecycle::default();
        assert_eq!(
            life.native_close_requested()[0],
            LifecycleEffect::CancelNativeClose
        );
        life.begin_review().unwrap();
        assert_eq!(
            life.native_close_requested(),
            vec![LifecycleEffect::CancelNativeClose]
        );
        life.begin_shutdown().unwrap();
        assert_eq!(
            life.native_close_requested(),
            vec![LifecycleEffect::CancelNativeClose]
        );
        assert_eq!(
            life.shutdown_succeeded().unwrap(),
            vec![LifecycleEffect::SendProgrammaticClose]
        );
        assert!(life.shutdown_succeeded().unwrap().is_empty());
        assert_eq!(
            life.native_close_requested(),
            vec![LifecycleEffect::AllowClose]
        );
        assert_eq!(
            life.native_close_requested(),
            vec![LifecycleEffect::AllowClose]
        );
    }
    #[test]
    fn failed_close_is_cancelled_and_can_retry_or_force() {
        let mut life = Lifecycle::default();
        life.request_exit();
        life.begin_review().unwrap();
        life.begin_shutdown().unwrap();
        life.shutdown_failed().unwrap();
        assert_eq!(
            life.native_close_requested(),
            vec![LifecycleEffect::CancelNativeClose]
        );
        assert_eq!(
            life.retry_shutdown().unwrap(),
            LifecycleEffect::BeginShutdown
        );
    }
    #[test]
    fn illegal_transitions_are_rejected() {
        let mut life = Lifecycle::default();
        assert_eq!(
            life.transition(LifecycleState::ReadyToClose),
            Err(IllegalTransition {
                from: LifecycleState::Running,
                to: LifecycleState::ReadyToClose
            })
        );
    }
    #[test]
    fn operations_have_explicit_shutdown_policy() {
        let mut r = OperationRegistry::default();
        r.register(1, OperationClass::CancellableRead);
        r.register(2, OperationClass::FinancialMutation);
        r.register(3, OperationClass::ReviewSensitive);
        r.register(4, OperationClass::CriticalExclusive);
        assert_eq!(
            r.shutdown_actions(),
            vec![
                (1, OperationShutdownAction::Cancel),
                (2, OperationShutdownAction::Wait),
                (3, OperationShutdownAction::Confirm),
                (4, OperationShutdownAction::Block)
            ]
        );
    }

    #[test]
    fn drafts_are_discarded_committed_or_returned_without_losing_input() {
        let clean_events = Arc::new(Mutex::new(Vec::new()));
        let valid_events = Arc::new(Mutex::new(Vec::new()));
        let invalid_events = Arc::new(Mutex::new(Vec::new()));
        let mut drafts = DraftRegistry::default();
        drafts.register(
            EditorId(1),
            Box::new(FakeDraft {
                status: DraftStatus::Clean,
                events: clean_events.clone(),
                fail: false,
            }),
        );
        drafts.register(
            EditorId(2),
            Box::new(FakeDraft {
                status: DraftStatus::ModifiedAndValid,
                events: valid_events.clone(),
                fail: false,
            }),
        );
        drafts.register(
            EditorId(3),
            Box::new(FakeDraft {
                status: DraftStatus::ModifiedAndInvalid,
                events: invalid_events.clone(),
                fail: false,
            }),
        );
        assert_eq!(drafts.review().unwrap(), Some(EditorId(3)));
        drafts.return_to_editor(EditorId(3));
        assert_eq!(*clean_events.lock().unwrap(), vec!["discard"]);
        assert_eq!(*valid_events.lock().unwrap(), vec!["commit"]);
        assert_eq!(*invalid_events.lock().unwrap(), vec!["focus"]);
        drafts.discard(EditorId(3));
        assert_eq!(*invalid_events.lock().unwrap(), vec!["focus", "discard"]);
    }

    #[test]
    fn failed_draft_commit_blocks_review() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let mut drafts = DraftRegistry::default();
        drafts.register(
            EditorId(1),
            Box::new(FakeDraft {
                status: DraftStatus::ModifiedAndValid,
                events,
                fail: true,
            }),
        );
        assert_eq!(drafts.review(), Err("commit failed".into()));
    }
}
