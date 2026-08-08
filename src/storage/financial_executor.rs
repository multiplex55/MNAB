//! One transaction, one audit intent, and one post-commit typed result per financial command.

use super::{
    protocol::{AffectedEntityId as A, MutationResult, UndoData},
    repository::{Repositories, UnitOfWork, UnitOfWorkFactory},
    sqlite_unit_of_work::SqliteUnitOfWorkFactory,
    worker::WorkerError,
};
use crate::app::{command::*, dispatcher::invalidations_for};
use crate::domain::Approval;
use rusqlite::Connection;

/// Envelope/generation verification happens before `begin`, so stale or malformed requests cannot
/// acquire a write transaction.
pub fn execute(
    connection: &mut Connection,
    envelope: &CommandEnvelope,
    request_budget: u64,
) -> Result<MutationResult, WorkerError> {
    if envelope.budget_generation != request_budget {
        return Err(safe("stale budget generation"));
    }
    let ApplicationAction::Financial(command) = &envelope.payload else {
        return Err(safe("invalid financial request"));
    };
    let mut factory = SqliteUnitOfWorkFactory::new(connection);
    execute_with(&mut factory, envelope, command)
}

fn execute_with<F: UnitOfWorkFactory>(
    factory: &mut F,
    envelope: &CommandEnvelope,
    command: &FinancialCommand,
) -> Result<MutationResult, WorkerError> {
    let mut work = factory.begin().map_err(repository)?;
    let (label, affected, undo) = apply(work.repositories(), command, envelope.correlation_id)?;
    let invalidations = invalidations_for(command);
    // Consuming commit makes a second commit impossible. No result exists before this succeeds.
    work.commit().map_err(repository)?;
    Ok(MutationResult {
        command_id: envelope.command_id,
        correlation_id: envelope.correlation_id,
        operation_label: label.to_owned(),
        affected_entity_ids: affected,
        undo,
        invalidations,
        navigation: None,
        focus_restoration: None,
        notice: None,
    })
}

fn apply<R: Repositories>(
    r: &mut R,
    c: &FinancialCommand,
    correlation: CorrelationId,
) -> Result<(&'static str, Vec<A>, Option<UndoData>), WorkerError> {
    let (label, ids, undo, entity, id) = match c {
        FinancialCommand::Account(AccountCommand::Create(v)) => {
            validate_name(&v.name)?;
            r.put_account(v).map_err(repository)?;
            (
                "Create account",
                vec![A::Account(v.id)],
                Some(UndoData::Command(FinancialCommand::Account(
                    AccountCommand::Close(v.id),
                ))),
                "account",
                v.id.to_string(),
            )
        }
        FinancialCommand::Account(AccountCommand::Update(v)) => {
            validate_name(&v.name)?;
            let old = r
                .account(v.id)
                .map_err(repository)?
                .ok_or_else(|| safe("account not found"))?;
            r.put_account(v).map_err(repository)?;
            (
                "Update account",
                vec![A::Account(v.id)],
                Some(UndoData::Command(FinancialCommand::Account(
                    AccountCommand::Update(old),
                ))),
                "account",
                v.id.to_string(),
            )
        }
        FinancialCommand::Account(AccountCommand::Close(id)) => {
            let mut old = r
                .account(*id)
                .map_err(repository)?
                .ok_or_else(|| safe("account not found"))?;
            let inverse = old.clone();
            old.closed = true;
            r.put_account(&old).map_err(repository)?;
            (
                "Close account",
                vec![A::Account(*id)],
                Some(UndoData::Command(FinancialCommand::Account(
                    AccountCommand::Update(inverse),
                ))),
                "account",
                id.to_string(),
            )
        }
        FinancialCommand::Category(CategoryCommand::Update(v)) => {
            validate_name(&v.name)?;
            let old = r.category(v.id).map_err(repository)?;
            r.put_category(v).map_err(repository)?;
            (
                "Update category",
                vec![A::Category(v.id)],
                old.map(|x| {
                    UndoData::Command(FinancialCommand::Category(CategoryCommand::Update(x)))
                }),
                "category",
                v.id.to_string(),
            )
        }
        FinancialCommand::Category(CategoryCommand::Delete(id)) => {
            if r.category_is_used(*id).map_err(repository)? {
                return Err(safe("category is in use"));
            }
            let old = r
                .category(*id)
                .map_err(repository)?
                .ok_or_else(|| safe("category not found"))?;
            r.delete_category(*id).map_err(repository)?;
            (
                "Delete category",
                vec![A::Category(*id)],
                Some(UndoData::Opaque(
                    serde_json::to_vec(&old).map_err(|_| safe("snapshot failed"))?,
                )),
                "category",
                id.to_string(),
            )
        }
        FinancialCommand::Assignment(AssignmentCommand::Set(v)) => {
            let old = r.assignment(v.category_id, v.month).map_err(repository)?;
            r.put_assignment(v).map_err(repository)?;
            (
                "Set assignment",
                vec![A::Assignment {
                    category: v.category_id,
                    month: v.month,
                }],
                old.map(|x| {
                    UndoData::Command(FinancialCommand::Assignment(AssignmentCommand::Set(x)))
                }),
                "assignment",
                format!(
                    "{}:{:04}-{:02}",
                    v.category_id,
                    v.month.year(),
                    v.month.month()
                ),
            )
        }
        FinancialCommand::Transaction(TransactionCommand::Save(v)) => {
            v.validate().map_err(|_| safe("invalid transaction"))?;
            let old = r.transaction(v.id).map_err(repository)?;
            r.put_transaction(v).map_err(repository)?;
            (
                "Save transaction",
                vec![A::Transaction(v.id), A::Account(v.account_id)],
                old.map(|x| {
                    UndoData::Command(FinancialCommand::Transaction(TransactionCommand::Save(x)))
                }),
                "transaction",
                v.id.to_string(),
            )
        }
        FinancialCommand::Transaction(TransactionCommand::Batch(batch)) => {
            let (preflight, prior, affected_ids) =
                crate::service::transaction_service::execute_batch(r, batch).map_err(repository)?;
            let mut affected = Vec::new();
            for old in &prior {
                affected.push(A::Transaction(old.id));
                affected.push(A::Account(old.account_id));
            }
            if let TransactionBatchAction::SetCategory(category) = &batch.action {
                affected.push(A::Category(*category));
            }
            affected.sort_by_key(|id| format!("{id:?}"));
            affected.dedup();
            let count = affected_ids.len();
            let label = match &batch.action {
                TransactionBatchAction::SetApproval(Approval::Approved) => "Approve transactions",
                TransactionBatchAction::SetApproval(Approval::Unapproved) => {
                    "Unapprove transactions"
                }
                TransactionBatchAction::SetCategory(_) => "Categorize transactions",
                TransactionBatchAction::SetPayee(_) => "Change transaction payees",
                TransactionBatchAction::SetClearance(_) => "Change cleared state",
                TransactionBatchAction::SetMemo(_) => "Change transaction memos",
                TransactionBatchAction::Void => "Void transactions",
                TransactionBatchAction::Delete => "Delete transactions",
                TransactionBatchAction::Restore(_) => "Restore transactions",
            };
            let inverse = TransactionBatchCommand {
                selection: TransactionBatchSelection::Explicit(
                    affected_ids.iter().copied().collect(),
                ),
                action: TransactionBatchAction::Restore(prior),
            };
            (
                label,
                affected,
                Some(UndoData::Command(FinancialCommand::Transaction(
                    TransactionCommand::Batch(inverse),
                ))),
                "transaction_batch",
                format!(
                    "{count}; eligible={}",
                    preflight
                        .count(crate::service::transaction_service::BatchPreflightReason::Eligible)
                ),
            )
        }
        FinancialCommand::Transaction(TransactionCommand::Delete {
            transaction_id,
            account_id,
            ..
        }) => {
            let old = r
                .transaction(*transaction_id)
                .map_err(repository)?
                .ok_or_else(|| safe("transaction not found"))?;
            if old.account_id != *account_id {
                return Err(safe("transaction account mismatch"));
            }
            r.delete_transaction(*transaction_id).map_err(repository)?;
            (
                "Delete transaction",
                vec![A::Transaction(*transaction_id), A::Account(*account_id)],
                Some(UndoData::Command(FinancialCommand::Transaction(
                    TransactionCommand::Save(old),
                ))),
                "transaction",
                transaction_id.to_string(),
            )
        }
        FinancialCommand::Payee(PayeeCommand::Update(v)) => {
            validate_name(&v.name)?;
            let old = r.payee(v.id).map_err(repository)?;
            r.put_payee(v).map_err(repository)?;
            (
                "Update payee",
                vec![A::Payee(v.id)],
                old.map(|x| UndoData::Command(FinancialCommand::Payee(PayeeCommand::Update(x)))),
                "payee",
                v.id.to_string(),
            )
        }
        FinancialCommand::Payee(PayeeCommand::Delete(id)) => {
            if r.payee_is_used(*id).map_err(repository)? {
                return Err(safe("payee is in use"));
            }
            let old = r
                .payee(*id)
                .map_err(repository)?
                .ok_or_else(|| safe("payee not found"))?;
            r.delete_payee(*id).map_err(repository)?;
            (
                "Delete payee",
                vec![A::Payee(*id)],
                Some(UndoData::Opaque(
                    serde_json::to_vec(&old).map_err(|_| safe("snapshot failed"))?,
                )),
                "payee",
                id.to_string(),
            )
        }
        FinancialCommand::Target(TargetCommand::Delete(id)) => {
            r.delete_target(*id).map_err(repository)?;
            (
                "Delete target",
                vec![A::Target(*id)],
                None,
                "target",
                id.to_string(),
            )
        }
        FinancialCommand::Schedule(ScheduleCommand::Delete(id)) => {
            r.delete_scheduled(*id).map_err(repository)?;
            (
                "Delete schedule",
                vec![A::Schedule(*id)],
                None,
                "schedule",
                id.to_string(),
            )
        }
        FinancialCommand::Inbox(InboxCommand::Resolve {
            item_id: crate::app::inbox::InboxItemId::FailedOperation(id),
            action: InboxAction::Dismiss,
        }) => {
            r.toggle_failure_dismissal(id).map_err(repository)?;
            (
                "Dismiss operation failure",
                vec![],
                // Dismissal is implemented as a toggle, so replaying the same semantic command is
                // a lossless inverse even if the projection disappeared before execution.
                Some(UndoData::Command(c.clone())),
                "operation_failure",
                id.clone(),
            )
        }
        _ => return Err(safe("financial command is not supported")),
    };
    r.append_audit(entity, &id, &format!("{label}; correlation={correlation}"))
        .map_err(repository)?;
    Ok((label, ids, undo))
}
fn validate_name(v: &str) -> Result<(), WorkerError> {
    if v.trim().is_empty() {
        Err(safe("name is required"))
    } else {
        Ok(())
    }
}
fn repository(e: crate::error::RepositoryError) -> WorkerError {
    WorkerError::Repository(format!("mutation persistence failed: {e}"))
}
fn safe(s: &str) -> WorkerError {
    WorkerError::Validation(s.into())
}
