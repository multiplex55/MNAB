use crate::app::{
    command::*,
    view_invalidation::{ViewInvalidation as V, ViewInvalidations},
};

#[derive(Default)]
pub struct ActionCollector(Vec<ApplicationAction>);
impl ActionCollector {
    pub fn push(&mut self, a: impl Into<ApplicationAction>) {
        self.0.push(a.into())
    }
    pub fn drain(&mut self) -> impl Iterator<Item = ApplicationAction> + '_ {
        self.0.drain(..)
    }
    pub fn into_actions(self) -> Vec<ApplicationAction> {
        self.0
    }
}
impl From<AppCommand> for ApplicationAction {
    fn from(v: AppCommand) -> Self {
        if v == AppCommand::Exit {
            Self::RequestExit
        } else {
            Self::Ui(v)
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DispatchError {
    ConfirmationRequired,
    IllegalTransition,
    SubmissionFailed,
    StaleGeneration,
    NotCancellable,
}
pub trait CommandSubmitter {
    fn submit(&mut self, command: &RuntimeCommand) -> Result<(), DispatchError>;
}

#[must_use]
pub fn requires_confirmation(c: &FinancialCommand) -> bool {
    matches!(
        c,
        FinancialCommand::Transaction(TransactionCommand::Delete { .. })
            | FinancialCommand::Category(CategoryCommand::Delete(_))
            | FinancialCommand::Payee(PayeeCommand::Delete(_))
            | FinancialCommand::Target(TargetCommand::Delete(_))
            | FinancialCommand::Schedule(ScheduleCommand::Delete(_))
    )
}

/// The only worker submission gateway. The request ID must already be associated, and the record
/// must already be in `Submitting`; this prevents enqueueing directly from queued/confirmation UI.
pub fn submit_command<S: CommandSubmitter>(
    submitter: &mut S,
    command: &mut RuntimeCommand,
) -> Result<(), DispatchError> {
    if command.status != CommandStatus::Submitting || command.worker_request_id.is_none() {
        return Err(DispatchError::IllegalTransition);
    }
    if requires_confirmation(match &command.envelope.payload {
        ApplicationAction::Financial(c) => c,
        _ => return Err(DispatchError::SubmissionFailed),
    }) && !matches!(command.confirmation, ConfirmationState::Confirmed(_))
    {
        return Err(DispatchError::ConfirmationRequired);
    }
    submitter.submit(command)?;
    command
        .transition(CommandStatus::Running)
        .map_err(|_| DispatchError::IllegalTransition)
}

#[must_use]
pub fn invalidations_for(c: &FinancialCommand) -> ViewInvalidations {
    match c {
        FinancialCommand::Inbox(_) => [
            V::Inbox,
            V::Accounts,
            V::AllAccountRegisters,
            V::Reports,
            V::Targets,
            V::Search,
            V::LookupData,
            V::Inspectors,
        ]
        .into_iter()
        .collect(),
        FinancialCommand::Assignment(AssignmentCommand::Set(a)) => [
            V::BudgetMonth(a.month),
            V::BudgetRolloverFrom(a.month),
            V::Reports,
            V::Targets,
            V::Inbox,
            V::Inspectors,
        ]
        .into_iter()
        .collect(),
        FinancialCommand::Transaction(TransactionCommand::Delete {
            account_id, month, ..
        }) => [
            V::AccountRegister(*account_id),
            V::BudgetMonth(*month),
            V::Reports,
            V::Targets,
            V::Inbox,
            V::Search,
            V::LookupData,
            V::Inspectors,
            V::BudgetRolloverFrom(*month),
        ]
        .into_iter()
        .collect(),
        FinancialCommand::Import(ImportCommand::Apply { account_id, .. }) => [
            V::AccountRegister(*account_id),
            V::Accounts,
            V::Inbox,
            V::Reports,
            V::Targets,
            V::Search,
            V::LookupData,
            V::Inspectors,
        ]
        .into_iter()
        .collect(),
        FinancialCommand::Account(_) => [
            V::Accounts,
            V::AllAccountRegisters,
            V::Reports,
            V::Search,
            V::LookupData,
            V::Inspectors,
        ]
        .into_iter()
        .collect(),
        _ => [
            V::Accounts,
            V::AllAccountRegisters,
            V::Reports,
            V::Targets,
            V::Inbox,
            V::Search,
            V::LookupData,
            V::Inspectors,
        ]
        .into_iter()
        .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn collector_is_semantic() {
        let mut c = ActionCollector::default();
        c.push(AppCommand::Import);
        assert_eq!(
            c.into_actions(),
            vec![ApplicationAction::Ui(AppCommand::Import)]
        );
    }
}
