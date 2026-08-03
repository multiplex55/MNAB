use crate::app::{
    command::{ApplicationAction, CommandEnvelope, FinancialCommand},
    view_invalidation::{ViewInvalidation, ViewInvalidations},
};

#[derive(Default)]
pub struct ActionCollector(Vec<ApplicationAction>);
impl ActionCollector {
    pub fn push(&mut self, action: impl Into<ApplicationAction>) {
        self.0.push(action.into());
    }
    pub fn drain(&mut self) -> impl Iterator<Item = ApplicationAction> + '_ {
        self.0.drain(..)
    }
    pub fn into_actions(self) -> Vec<ApplicationAction> {
        self.0
    }
}
impl From<crate::app::command::AppCommand> for ApplicationAction {
    fn from(value: crate::app::command::AppCommand) -> Self {
        Self::Ui(value)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DispatchError {
    ConfirmationRequired,
    SubmissionFailed,
}

pub trait CommandSubmitter {
    fn submit(&mut self, envelope: &CommandEnvelope) -> Result<(), DispatchError>;
}

pub fn requires_confirmation(command: &FinancialCommand) -> bool {
    matches!(command, FinancialCommand::DeleteTransaction { .. })
}

pub fn validate_confirmation(envelope: &CommandEnvelope) -> Result<(), DispatchError> {
    if matches!(&envelope.payload, ApplicationAction::Financial(c) if requires_confirmation(c))
        && envelope.confirmation_token.is_none()
    {
        Err(DispatchError::ConfirmationRequired)
    } else {
        Ok(())
    }
}

/// Submits at the worker boundary and records undo state only after acceptance.
/// This ordering is the invariant that prevents failed dispatches entering history.
pub fn submit_financial<S: CommandSubmitter>(
    submitter: &mut S,
    envelope: &CommandEnvelope,
    history: &mut crate::app::command::CommandHistory<FinancialCommand>,
    history_entry: crate::app::command::HistoryEntry<FinancialCommand>,
) -> Result<ViewInvalidations, DispatchError> {
    validate_confirmation(envelope)?;
    submitter.submit(envelope)?;
    history.record_success(history_entry);
    let ApplicationAction::Financial(command) = &envelope.payload else {
        return Ok(ViewInvalidations::default());
    };
    Ok(invalidations_for(command))
}

#[must_use]
pub fn invalidations_for(command: &FinancialCommand) -> ViewInvalidations {
    use ViewInvalidation as V;
    match command {
        FinancialCommand::Assign { month, .. } => {
            [V::BudgetMonth(*month), V::Reports, V::Inspectors]
                .into_iter()
                .collect()
        }
        FinancialCommand::DeleteTransaction {
            account_id, month, ..
        } => [
            V::AccountRegister(*account_id),
            V::BudgetMonth(*month),
            V::Reports,
            V::Search,
            V::Inspectors,
        ]
        .into_iter()
        .collect(),
        FinancialCommand::AddAccount { .. } => {
            [V::Accounts, V::AllAccountRegisters].into_iter().collect()
        }
        FinancialCommand::Import { account_id } => {
            [V::AccountRegister(*account_id), V::Inbox, V::Reports]
                .into_iter()
                .collect()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        app::command::{AppCommand, CommandHistory, HistoryEntry},
        domain::{AccountId, BudgetMonth, TransactionId},
    };

    struct FakeSubmitter {
        fail: bool,
        calls: usize,
    }
    impl CommandSubmitter for FakeSubmitter {
        fn submit(&mut self, _: &CommandEnvelope) -> Result<(), DispatchError> {
            self.calls += 1;
            if self.fail {
                Err(DispatchError::SubmissionFailed)
            } else {
                Ok(())
            }
        }
    }
    fn deletion() -> FinancialCommand {
        FinancialCommand::DeleteTransaction {
            transaction_id: TransactionId::new(),
            account_id: AccountId::new(),
            month: BudgetMonth::new(2026, 8).unwrap(),
        }
    }
    fn envelope(command: FinancialCommand, confirmed: bool) -> CommandEnvelope {
        CommandEnvelope {
            command_id: 1,
            correlation_id: 1,
            budget_generation: 2,
            payload: ApplicationAction::Financial(command),
            confirmation_token: confirmed.then_some(4),
            focus_restoration_id: None,
        }
    }
    #[test]
    fn collector_preserves_widget_order() {
        let mut collector = ActionCollector::default();
        collector.push(AppCommand::AddAccount);
        collector.push(AppCommand::Import);
        assert_eq!(
            collector.into_actions(),
            vec![
                ApplicationAction::Ui(AppCommand::AddAccount),
                ApplicationAction::Ui(AppCommand::Import)
            ]
        );
    }
    #[test]
    fn confirmation_precedes_submission() {
        let mut fake = FakeSubmitter {
            fail: false,
            calls: 0,
        };
        let command = deletion();
        let mut history = CommandHistory::new(5);
        let result = submit_financial(
            &mut fake,
            &envelope(command.clone(), false),
            &mut history,
            HistoryEntry {
                label: "delete".into(),
                command: command.clone(),
                inverse: command,
            },
        );
        assert_eq!(result, Err(DispatchError::ConfirmationRequired));
        assert_eq!(fake.calls, 0);
    }
    #[test]
    fn failed_submission_does_not_change_history() {
        let mut fake = FakeSubmitter {
            fail: true,
            calls: 0,
        };
        let command = deletion();
        let mut history = CommandHistory::new(5);
        assert_eq!(
            submit_financial(
                &mut fake,
                &envelope(command.clone(), true),
                &mut history,
                HistoryEntry {
                    label: "delete".into(),
                    command: command.clone(),
                    inverse: command
                }
            ),
            Err(DispatchError::SubmissionFailed)
        );
        assert_eq!(history.undo_len(), 0);
    }
}
