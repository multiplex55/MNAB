//! Commands for schedules and their durable review occurrences.
use crate::domain::*;
use std::collections::HashMap;

#[derive(Clone, Debug, Default)]
pub struct ScheduleLedger {
    pub schedules: HashMap<ScheduledTransactionId, ScheduledTransaction>,
    pub occurrences: OccurrenceStore,
    pub transactions: HashMap<TransactionId, Transaction>,
}

/// Stages linked transaction and disposition writes against a clone, making replacement the
/// single commit point. Persistence adapters can use the same command inside a unit of work.
pub struct ScheduleService<'a> {
    ledger: &'a mut ScheduleLedger,
}
impl<'a> ScheduleService<'a> {
    pub fn new(ledger: &'a mut ScheduleLedger) -> Self {
        Self { ledger }
    }
    pub fn create(&mut self, schedule: ScheduledTransaction) {
        self.ledger.schedules.insert(schedule.id, schedule);
    }
    pub fn activate(&mut self, id: ScheduledTransactionId) -> Result<(), ScheduleError> {
        self.schedule_mut(id)?.activate();
        Ok(())
    }
    pub fn deactivate(&mut self, id: ScheduledTransactionId) -> Result<(), ScheduleError> {
        self.schedule_mut(id)?.deactivate();
        Ok(())
    }
    fn schedule_mut(
        &mut self,
        id: ScheduledTransactionId,
    ) -> Result<&mut ScheduledTransaction, ScheduleError> {
        self.ledger
            .schedules
            .get_mut(&id)
            .ok_or(ScheduleError::ScheduleNotFound)
    }
    pub fn refresh(
        &mut self,
        id: ScheduledTransactionId,
        today: time::Date,
        look_ahead_days: u32,
    ) -> Result<Vec<ScheduledOccurrenceId>, ScheduleError> {
        let schedule = self
            .ledger
            .schedules
            .get(&id)
            .cloned()
            .ok_or(ScheduleError::ScheduleNotFound)?;
        self.ledger
            .occurrences
            .refresh(&schedule, today, look_ahead_days)
    }
    pub fn enter_now(
        &mut self,
        id: ScheduledOccurrenceId,
        budget_id: BudgetId,
    ) -> Result<Transaction, ScheduleError> {
        let occurrence = self
            .ledger
            .occurrences
            .occurrence(id)
            .filter(|o| o.disposition == OccurrenceDisposition::Pending)
            .cloned()
            .ok_or(ScheduleError::OccurrenceUnavailable)?;
        let schedule = self
            .ledger
            .schedules
            .get(&occurrence.schedule_id)
            .ok_or(ScheduleError::ScheduleNotFound)?;
        let transaction = Transaction {
            id: TransactionId::new(),
            budget_id,
            account_id: schedule.account_id,
            date: TransactionDate(occurrence.date),
            payee_id: occurrence.payee_id,
            amount: occurrence.amount,
            memo: Some("Entered from schedule".into()),
            clearance: Clearance::Uncleared,
            approval: Approval::Approved,
            body: occurrence.category_id.map_or(
                TransactionBody::OpeningBalance { category_id: None },
                |category_id| TransactionBody::Categorized { category_id },
            ),
            archived: false,
            voided: false,
        };
        let mut staged = self.ledger.clone();
        staged
            .transactions
            .insert(transaction.id, transaction.clone());
        staged.occurrences.enter_now(id, transaction.id)?;
        *self.ledger = staged;
        Ok(transaction)
    }
}
