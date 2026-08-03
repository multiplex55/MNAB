//! Recurring transaction templates and explicitly pending occurrences.
use super::{
    AccountId, CategoryId, Money, PayeeId, ScheduledOccurrenceId, ScheduledTransactionId,
    TransactionId,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use thiserror::Error;
use time::{Date, Duration, Month};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum Recurrence {
    Daily,
    Weekly,
    EveryTwoWeeks,
    Monthly,
    Yearly,
    CustomDays(u32),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScheduledTransaction {
    pub id: ScheduledTransactionId,
    pub account_id: AccountId,
    pub start_date: Date,
    pub amount: Money,
    pub payee_id: Option<PayeeId>,
    pub category_id: Option<CategoryId>,
    pub recurrence: Recurrence,
    /// Inclusive: an occurrence exactly on this date is generated.
    pub end_date: Option<Date>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum OccurrenceDisposition {
    Pending,
    Skipped,
    Dismissed,
    Entered(TransactionId),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScheduledOccurrence {
    pub id: ScheduledOccurrenceId,
    pub schedule_id: ScheduledTransactionId,
    pub sequence: u32,
    pub date: Date,
    pub amount: Money,
    pub payee_id: Option<PayeeId>,
    pub category_id: Option<CategoryId>,
    pub disposition: OccurrenceDisposition,
}

#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub enum ScheduleError {
    #[error("custom interval must contain at least one day")]
    InvalidCustomInterval,
    #[error("schedule end date precedes its start date")]
    EndBeforeStart,
    #[error("date calculation is outside the supported range")]
    DateOutOfRange,
    #[error("look-ahead window cannot be negative")]
    InvalidLookAhead,
    #[error("occurrence was not found or is no longer pending")]
    OccurrenceUnavailable,
}

impl ScheduledTransaction {
    pub fn new(
        account_id: AccountId,
        start_date: Date,
        amount: Money,
        recurrence: Recurrence,
        end_date: Option<Date>,
    ) -> Result<Self, ScheduleError> {
        if matches!(recurrence, Recurrence::CustomDays(0)) {
            return Err(ScheduleError::InvalidCustomInterval);
        }
        if end_date.is_some_and(|d| d < start_date) {
            return Err(ScheduleError::EndBeforeStart);
        }
        Ok(Self {
            id: ScheduledTransactionId::new(),
            account_id,
            start_date,
            amount,
            payee_id: None,
            category_id: None,
            recurrence,
            end_date,
        })
    }
    pub fn occurrence_date(&self, sequence: u32) -> Result<Option<Date>, ScheduleError> {
        let date = match self.recurrence {
            Recurrence::Daily => add_days(self.start_date, i64::from(sequence))?,
            Recurrence::Weekly => add_days(self.start_date, i64::from(sequence) * 7)?,
            Recurrence::EveryTwoWeeks => add_days(self.start_date, i64::from(sequence) * 14)?,
            Recurrence::CustomDays(days) if days > 0 => {
                add_days(self.start_date, i64::from(sequence) * i64::from(days))?
            }
            Recurrence::CustomDays(_) => return Err(ScheduleError::InvalidCustomInterval),
            Recurrence::Monthly => add_months_clamped(self.start_date, sequence)?,
            Recurrence::Yearly => add_months_clamped(
                self.start_date,
                sequence
                    .checked_mul(12)
                    .ok_or(ScheduleError::DateOutOfRange)?,
            )?,
        };
        Ok((self.end_date.is_none_or(|end| date <= end)).then_some(date))
    }
}

/// Month/year recurrence is anchored to the original day and clamps to the destination month's
/// final day. Thus Jan 31 -> Feb 28/29 -> Mar 31, and Feb 29 yearly -> Feb 28 -> Feb 28 -> Feb 29.
fn add_months_clamped(date: Date, count: u32) -> Result<Date, ScheduleError> {
    let absolute = i64::from(date.year())
        .checked_mul(12)
        .and_then(|v| v.checked_add(i64::from(u8::from(date.month())) - 1))
        .and_then(|v| v.checked_add(i64::from(count)))
        .ok_or(ScheduleError::DateOutOfRange)?;
    let year = i32::try_from(absolute.div_euclid(12)).map_err(|_| ScheduleError::DateOutOfRange)?;
    let month_number =
        u8::try_from(absolute.rem_euclid(12) + 1).map_err(|_| ScheduleError::DateOutOfRange)?;
    let month = Month::try_from(month_number).map_err(|_| ScheduleError::DateOutOfRange)?;
    let last = days_in_month(year, month);
    Date::from_calendar_date(year, month, date.day().min(last))
        .map_err(|_| ScheduleError::DateOutOfRange)
}
fn add_days(date: Date, days: i64) -> Result<Date, ScheduleError> {
    date.checked_add(Duration::days(days))
        .ok_or(ScheduleError::DateOutOfRange)
}
fn days_in_month(year: i32, month: Month) -> u8 {
    let next = if month == Month::December {
        Date::from_calendar_date(year + 1, Month::January, 1)
    } else {
        Date::from_calendar_date(year, month.next(), 1)
    };
    next.expect("supported date")
        .previous_day()
        .expect("day before first")
        .day()
}

#[derive(Clone, Debug, Default)]
pub struct OccurrenceStore {
    values: BTreeMap<(ScheduledTransactionId, u32), ScheduledOccurrence>,
}
impl OccurrenceStore {
    /// Creates pending review items only. Repeated refreshes reuse `(schedule, sequence)` identity.
    pub fn refresh(
        &mut self,
        schedule: &ScheduledTransaction,
        today: Date,
        look_ahead_days: u32,
    ) -> Result<Vec<ScheduledOccurrenceId>, ScheduleError> {
        let through = today
            .checked_add(Duration::days(i64::from(look_ahead_days)))
            .ok_or(ScheduleError::InvalidLookAhead)?;
        let mut created = vec![];
        for sequence in 0.. {
            let Some(date) = schedule.occurrence_date(sequence)? else {
                break;
            };
            if date > through {
                break;
            }
            if date < today {
                continue;
            }
            let key = (schedule.id, sequence);
            if let std::collections::btree_map::Entry::Vacant(slot) = self.values.entry(key) {
                let value = ScheduledOccurrence {
                    id: ScheduledOccurrenceId::new(),
                    schedule_id: schedule.id,
                    sequence,
                    date,
                    amount: schedule.amount,
                    payee_id: schedule.payee_id,
                    category_id: schedule.category_id,
                    disposition: OccurrenceDisposition::Pending,
                };
                created.push(value.id);
                slot.insert(value);
            }
        }
        Ok(created)
    }
    pub fn pending(&self) -> impl Iterator<Item = &ScheduledOccurrence> {
        self.values
            .values()
            .filter(|v| v.disposition == OccurrenceDisposition::Pending)
    }
    fn pending_mut(
        &mut self,
        id: ScheduledOccurrenceId,
    ) -> Result<&mut ScheduledOccurrence, ScheduleError> {
        self.values
            .values_mut()
            .find(|v| v.id == id && v.disposition == OccurrenceDisposition::Pending)
            .ok_or(ScheduleError::OccurrenceUnavailable)
    }
    pub fn enter_now(
        &mut self,
        id: ScheduledOccurrenceId,
        transaction_id: TransactionId,
    ) -> Result<(), ScheduleError> {
        self.pending_mut(id)?.disposition = OccurrenceDisposition::Entered(transaction_id);
        Ok(())
    }
    pub fn skip(&mut self, id: ScheduledOccurrenceId) -> Result<(), ScheduleError> {
        self.pending_mut(id)?.disposition = OccurrenceDisposition::Skipped;
        Ok(())
    }
    pub fn dismiss(&mut self, id: ScheduledOccurrenceId) -> Result<(), ScheduleError> {
        self.pending_mut(id)?.disposition = OccurrenceDisposition::Dismissed;
        Ok(())
    }
    pub fn modify_occurrence(
        &mut self,
        id: ScheduledOccurrenceId,
        date: Date,
        amount: Money,
    ) -> Result<(), ScheduleError> {
        let v = self.pending_mut(id)?;
        v.date = date;
        v.amount = amount;
        Ok(())
    }
    pub fn modify_series(
        &mut self,
        schedule: &mut ScheduledTransaction,
        amount: Money,
        recurrence: Recurrence,
        end_date: Option<Date>,
    ) -> Result<(), ScheduleError> {
        if matches!(recurrence, Recurrence::CustomDays(0)) {
            return Err(ScheduleError::InvalidCustomInterval);
        }
        if end_date.is_some_and(|d| d < schedule.start_date) {
            return Err(ScheduleError::EndBeforeStart);
        }
        schedule.amount = amount;
        schedule.recurrence = recurrence;
        schedule.end_date = end_date;
        for value in self.values.values_mut().filter(|v| {
            v.schedule_id == schedule.id && v.disposition == OccurrenceDisposition::Pending
        }) {
            value.amount = amount;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::date;
    fn schedule(start: Date, recurrence: Recurrence, end: Option<Date>) -> ScheduledTransaction {
        ScheduledTransaction::new(
            AccountId::new(),
            start,
            Money::from_minor_units(-10),
            recurrence,
            end,
        )
        .unwrap()
    }
    #[test]
    fn recurrence_sequences_and_clamping() {
        let monthly = schedule(date!(2024 - 01 - 31), Recurrence::Monthly, None);
        assert_eq!(
            monthly.occurrence_date(1).unwrap(),
            Some(date!(2024 - 02 - 29))
        );
        assert_eq!(
            monthly.occurrence_date(2).unwrap(),
            Some(date!(2024 - 03 - 31))
        );
        let yearly = schedule(date!(2024 - 02 - 29), Recurrence::Yearly, None);
        assert_eq!(
            yearly.occurrence_date(1).unwrap(),
            Some(date!(2025 - 02 - 28))
        );
        assert_eq!(
            yearly.occurrence_date(4).unwrap(),
            Some(date!(2028 - 02 - 29))
        );
        for (r, expected) in [
            (Recurrence::Daily, date!(2026 - 01 - 02)),
            (Recurrence::Weekly, date!(2026 - 01 - 08)),
            (Recurrence::EveryTwoWeeks, date!(2026 - 01 - 15)),
            (Recurrence::CustomDays(3), date!(2026 - 01 - 04)),
        ] {
            assert_eq!(
                schedule(date!(2026 - 01 - 01), r, None)
                    .occurrence_date(1)
                    .unwrap(),
                Some(expected)
            );
        }
    }
    #[test]
    fn inclusive_end_and_idempotent_refresh_actions() {
        let mut s = schedule(
            date!(2026 - 01 - 01),
            Recurrence::Daily,
            Some(date!(2026 - 01 - 02)),
        );
        assert_eq!(s.occurrence_date(1).unwrap(), Some(date!(2026 - 01 - 02)));
        assert_eq!(s.occurrence_date(2).unwrap(), None);
        let mut store = OccurrenceStore::default();
        assert_eq!(
            store.refresh(&s, date!(2026 - 01 - 01), 1).unwrap().len(),
            2
        );
        assert!(
            store
                .refresh(&s, date!(2026 - 01 - 01), 1)
                .unwrap()
                .is_empty()
        );
        let id = store.pending().next().unwrap().id;
        store
            .modify_occurrence(id, date!(2026 - 01 - 01), Money::ZERO)
            .unwrap();
        store.skip(id).unwrap();
        assert_eq!(store.pending().count(), 1);
        store
            .modify_series(
                &mut s,
                Money::from_minor_units(-20),
                Recurrence::Weekly,
                None,
            )
            .unwrap();
        assert_eq!(
            store.pending().next().unwrap().amount,
            Money::from_minor_units(-20)
        );
    }
    #[test]
    fn custom_validation() {
        assert_eq!(
            ScheduledTransaction::new(
                AccountId::new(),
                date!(2026 - 01 - 01),
                Money::ZERO,
                Recurrence::CustomDays(0),
                None
            ),
            Err(ScheduleError::InvalidCustomInterval)
        );
    }
}
