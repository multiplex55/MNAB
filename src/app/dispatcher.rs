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
impl From<crate::app::command::ReportAction> for ApplicationAction {
    fn from(value: crate::app::command::ReportAction) -> Self {
        Self::Report(value)
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
    let mut invalidations: ViewInvalidations = match c {
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
        FinancialCommand::Assignment(assignment) => {
            let month = match assignment {
                AssignmentCommand::Set(a) => a.month,
                AssignmentCommand::Remove { month, .. } => *month,
                AssignmentCommand::Batch(batch) => batch.month,
            };
            [
                V::BudgetMonth(month),
                V::BudgetRolloverFrom(month),
                V::Reports,
                V::Targets,
                V::Inbox,
                V::Inspectors,
            ]
            .into_iter()
            .collect()
        }
        FinancialCommand::Transaction(TransactionCommand::Save(transaction)) => {
            let month = crate::domain::BudgetMonth::new(
                transaction.date.0.year(),
                u8::from(transaction.date.0.month()),
            )
            .expect("transaction date has a valid month");
            [
                V::AccountRegister(transaction.account_id),
                V::Accounts,
                V::BudgetMonth(month),
                V::BudgetRolloverFrom(month),
                V::Reports,
                V::Targets,
                V::Inbox,
                V::Search,
                V::LookupData,
                V::Inspectors,
            ]
            .into_iter()
            .collect()
        }
        FinancialCommand::Transaction(TransactionCommand::SaveTransfer {
            source,
            destination,
        }) => {
            let source_month = crate::domain::BudgetMonth::new(
                source.date.0.year(),
                u8::from(source.date.0.month()),
            )
            .expect("transaction date has a valid month");
            let destination_month = crate::domain::BudgetMonth::new(
                destination.date.0.year(),
                u8::from(destination.date.0.month()),
            )
            .expect("transaction date has a valid month");
            let mut values: ViewInvalidations = [
                V::AccountRegister(source.account_id),
                V::AccountRegister(destination.account_id),
                V::Accounts,
                V::BudgetMonth(source_month),
                V::BudgetRolloverFrom(source_month),
                V::Reports,
                V::Targets,
                V::Inbox,
                V::Search,
                V::LookupData,
                V::Inspectors,
            ]
            .into_iter()
            .collect();
            values.insert(V::BudgetMonth(destination_month));
            values.insert(V::BudgetRolloverFrom(destination_month));
            values
        }
        FinancialCommand::Transaction(TransactionCommand::Delete {
            account_id, month, ..
        }) => [
            V::AccountRegister(*account_id),
            V::Accounts,
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
        FinancialCommand::Transaction(TransactionCommand::Batch(_)) => [
            V::AllAccountRegisters,
            V::Accounts,
            V::Reports,
            V::Targets,
            V::Inbox,
            V::Search,
            V::LookupData,
            V::Inspectors,
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
        FinancialCommand::Category(command) => {
            let category = match command {
                CategoryCommand::Update(v) => Some(v.id),
                CategoryCommand::Delete(id) => Some(*id),
                CategoryCommand::ReorderCategory { id, .. } => Some(*id),
                CategoryCommand::Merge { source, .. } => Some(*source),
                CategoryCommand::CreateGroup(_) | CategoryCommand::ReorderGroup { .. } => None,
            };
            let mut values: ViewInvalidations = [
                V::CategoryCatalog,
                V::Reports,
                V::Targets,
                V::Inspectors,
                V::LookupData,
                V::Search,
                V::SavedViewDiagnostics,
                V::AllAccountRegisters,
            ]
            .into_iter()
            .collect();
            if let Some(id) = category {
                values.insert(V::CategoryDetail(id));
            }
            values
        }
        FinancialCommand::Target(command) => {
            let mut values: ViewInvalidations =
                [V::Targets, V::Reports, V::Inspectors, V::CategoryCatalog]
                    .into_iter()
                    .collect();
            if let TargetCommand::Save(target) = command {
                let id = match target.association {
                    crate::domain::TargetAssociation::Category(id)
                    | crate::domain::TargetAssociation::CreditCard {
                        payment_category_id: id,
                        ..
                    } => id,
                };
                values.insert(V::CategoryDetail(id));
            }
            values
        }
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
    };
    // Every ledger or label mutation can change membership, amounts, or a
    // materialized label in the cross-account projection.
    invalidations.insert(V::AllTransactions);
    invalidations
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
    #[test]
    fn category_mutation_invalidates_dependent_projections_not_the_whole_app() {
        let id = crate::domain::CategoryId::new();
        let group = crate::domain::CategoryGroupId::new();
        let values = invalidations_for(&FinancialCommand::Category(CategoryCommand::Update(
            crate::domain::Category {
                id,
                group_id: group,
                name: "Food".into(),
                hidden: false,
                archived: false,
            },
        )));
        for expected in [
            V::CategoryCatalog,
            V::CategoryDetail(id),
            V::Reports,
            V::Inspectors,
            V::SavedViewDiagnostics,
            V::AllTransactions,
        ] {
            assert!(
                values.iter().any(|v| v == &expected),
                "missing {expected:?}"
            );
        }
        assert!(
            !values
                .iter()
                .any(|v| matches!(v, V::Accounts | V::Inbox | V::Schedules))
        );
    }
}
