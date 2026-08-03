//! Atomic assignment commands and their active-session inverses.
use crate::domain::{BudgetMonth, CategoryId, Money, MoneyError};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DistributionRule {
    Equal,
    InOrder,
}
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Assignments {
    values: HashMap<(CategoryId, BudgetMonth), Money>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UndoAssignment {
    before: Vec<((CategoryId, BudgetMonth), Option<Money>)>,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AutoAssignStrategy {
    Underfunded,
    ScheduledThisMonth,
    AssignedLastMonth,
    SpentLastMonth,
    AverageAssigned { periods: u32 },
    AverageSpent { periods: u32 },
    ResetAssigned,
    ResetAvailable,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AutoAssignInput {
    pub category_id: CategoryId,
    pub assigned: Money,
    pub available: Money,
    pub underfunded: Money,
    pub scheduled_this_month: Money,
    pub assigned_history: Vec<Money>,
    pub spent_history: Vec<Money>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssignmentChange {
    pub category_id: CategoryId,
    pub before: Money,
    pub after: Money,
    pub delta: Money,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AutoAssignPreview {
    pub month: BudgetMonth,
    pub changes: Vec<AssignmentChange>,
    pub total_assignment: Money,
    pub remaining_rta: Money,
    pub warnings: Vec<String>,
    pub inverse_commands: Vec<(CategoryId, Money)>,
}
impl Assignments {
    #[must_use]
    pub fn get(&self, id: CategoryId, month: BudgetMonth) -> Money {
        self.values
            .get(&(id, month))
            .copied()
            .unwrap_or(Money::ZERO)
    }
    fn apply(&mut self, changes: Vec<((CategoryId, BudgetMonth), Money)>) -> UndoAssignment {
        let before = changes
            .iter()
            .map(|(k, _)| (*k, self.values.get(k).copied()))
            .collect();
        for (k, v) in changes {
            if v == Money::ZERO {
                self.values.remove(&k);
            } else {
                self.values.insert(k, v);
            }
        }
        UndoAssignment { before }
    }
    /// Applies a previously reviewed preview in one all-or-nothing mutation.
    pub fn apply_preview(
        &mut self,
        preview: &AutoAssignPreview,
    ) -> Result<UndoAssignment, MoneyError> {
        // Revalidate the entire preview before touching the map (including stale previews).
        let mut changes = Vec::with_capacity(preview.changes.len());
        let mut total = Money::ZERO;
        for change in &preview.changes {
            if self.get(change.category_id, preview.month) != change.before {
                return Err(MoneyError::Invalid);
            }
            total = total.checked_add(change.delta)?;
            if change.before.checked_add(change.delta)? != change.after {
                return Err(MoneyError::Invalid);
            }
            changes.push(((change.category_id, preview.month), change.after));
        }
        if total != preview.total_assignment {
            return Err(MoneyError::Invalid);
        }
        Ok(self.apply(changes))
    }
    pub fn replace(&mut self, id: CategoryId, month: BudgetMonth, value: Money) -> UndoAssignment {
        self.apply(vec![((id, month), value)])
    }
    pub fn add(
        &mut self,
        id: CategoryId,
        month: BudgetMonth,
        delta: Money,
    ) -> Result<UndoAssignment, MoneyError> {
        let value = self.get(id, month).checked_add(delta)?;
        Ok(self.replace(id, month, value))
    }
    pub fn reset(&mut self, id: CategoryId, month: BudgetMonth) -> UndoAssignment {
        self.replace(id, month, Money::ZERO)
    }
    pub fn move_money(
        &mut self,
        from: CategoryId,
        to: CategoryId,
        month: BudgetMonth,
        amount: Money,
    ) -> Result<UndoAssignment, MoneyError> {
        let source = self.get(from, month).checked_sub(amount)?;
        let destination = self.get(to, month).checked_add(amount)?;
        Ok(self.apply(vec![((from, month), source), ((to, month), destination)]))
    }
    pub fn assign_all(
        &mut self,
        id: CategoryId,
        month: BudgetMonth,
        rta: Money,
    ) -> Result<UndoAssignment, MoneyError> {
        self.add(id, month, rta)
    }
    pub fn distribute(
        &mut self,
        ids: &[CategoryId],
        month: BudgetMonth,
        amount: Money,
        rule: DistributionRule,
    ) -> Result<UndoAssignment, MoneyError> {
        if ids.is_empty() {
            return Ok(UndoAssignment { before: vec![] });
        }
        let n = i64::try_from(ids.len()).map_err(|_| MoneyError::Overflow)?;
        let base = Money::from_minor_units(amount.minor_units() / n);
        let mut remainder = amount.minor_units() % n;
        let mut changes = Vec::with_capacity(ids.len());
        for id in ids {
            let extra = if rule == DistributionRule::InOrder && remainder != 0 {
                let v = remainder.signum();
                remainder -= v;
                v
            } else {
                0
            };
            changes.push((
                (*id, month),
                self.get(*id, month)
                    .checked_add(Money::from_minor_units(base.minor_units() + extra))?,
            ));
        }
        Ok(self.apply(changes))
    }
    pub fn undo(&mut self, undo: UndoAssignment) {
        for (k, v) in undo.before {
            if let Some(value) = v {
                self.values.insert(k, value);
            } else {
                self.values.remove(&k);
            }
        }
    }
}

pub fn propose_auto_assign(
    month: BudgetMonth,
    rta: Money,
    inputs: &[AutoAssignInput],
    strategy: AutoAssignStrategy,
) -> Result<AutoAssignPreview, MoneyError> {
    let mut changes = vec![];
    let mut warnings = vec![];
    let mut total = Money::ZERO;
    for input in inputs {
        let delta = match strategy {
            AutoAssignStrategy::Underfunded => input.underfunded.max(Money::ZERO),
            AutoAssignStrategy::ScheduledThisMonth => input.scheduled_this_month.max(Money::ZERO),
            AutoAssignStrategy::AssignedLastMonth => input
                .assigned_history
                .first()
                .copied()
                .unwrap_or(Money::ZERO),
            AutoAssignStrategy::SpentLastMonth => {
                positive_spending(input.spent_history.first().copied().unwrap_or(Money::ZERO))?
            }
            AutoAssignStrategy::AverageAssigned { periods } => {
                average(&input.assigned_history, periods)?
            }
            AutoAssignStrategy::AverageSpent { periods } => {
                average_spent(&input.spent_history, periods)?
            }
            AutoAssignStrategy::ResetAssigned => input.assigned.checked_neg()?,
            AutoAssignStrategy::ResetAvailable => {
                // Available changes one-for-one with assignment in this isolated month model.
                // Positive activity/carry-over may make the inverse an unassignment, which is valid.
                input.available.checked_neg()?
            }
        };
        let after = input.assigned.checked_add(delta)?;
        total = total.checked_add(delta)?;
        changes.push(AssignmentChange {
            category_id: input.category_id,
            before: input.assigned,
            after,
            delta,
        });
    }
    let remaining_rta = rta.checked_sub(total)?;
    if remaining_rta < Money::ZERO {
        warnings.push(format!(
            "Proposal exceeds Ready to Assign by {}",
            remaining_rta.checked_neg()?
        ));
    }
    let inverse_commands = changes.iter().map(|c| (c.category_id, c.before)).collect();
    Ok(AutoAssignPreview {
        month,
        changes,
        total_assignment: total,
        remaining_rta,
        warnings,
        inverse_commands,
    })
}
fn positive_spending(value: Money) -> Result<Money, MoneyError> {
    if value < Money::ZERO {
        value.checked_neg()
    } else {
        Ok(value)
    }
}
fn average(values: &[Money], periods: u32) -> Result<Money, MoneyError> {
    if periods == 0 {
        return Err(MoneyError::Invalid);
    }
    let count = usize::try_from(periods)
        .map_err(|_| MoneyError::Overflow)?
        .min(values.len());
    if count == 0 {
        return Ok(Money::ZERO);
    }
    let sum = values
        .iter()
        .take(count)
        .try_fold(Money::ZERO, |a, v| a.checked_add(*v))?;
    Ok(Money::from_minor_units(
        sum.minor_units() / i64::try_from(count).map_err(|_| MoneyError::Overflow)?,
    ))
}
fn average_spent(values: &[Money], periods: u32) -> Result<Money, MoneyError> {
    let normalized: Result<Vec<_>, _> = values.iter().copied().map(positive_spending).collect();
    average(&normalized?, periods)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn m(v: i64) -> Money {
        Money::from_minor_units(v)
    }
    #[test]
    fn move_conserves_and_undo_restores() {
        let month = BudgetMonth::new(2026, 1).unwrap();
        let (a, b) = (CategoryId::new(), CategoryId::new());
        let mut s = Assignments::default();
        s.replace(a, month, m(700));
        s.replace(b, month, m(300));
        let undo = s.move_money(a, b, month, m(250)).unwrap();
        assert_eq!(
            s.get(a, month).checked_add(s.get(b, month)).unwrap(),
            m(1000)
        );
        s.undo(undo);
        assert_eq!((s.get(a, month), s.get(b, month)), (m(700), m(300)));
    }
    #[test]
    fn overflow_does_not_partially_apply_batch() {
        let month = BudgetMonth::new(2026, 1).unwrap();
        let (a, b) = (CategoryId::new(), CategoryId::new());
        let mut s = Assignments::default();
        s.replace(a, month, m(10));
        s.replace(b, month, m(i64::MAX));
        assert!(s.move_money(a, b, month, m(1)).is_err());
        assert_eq!((s.get(a, month), s.get(b, month)), (m(10), m(i64::MAX)));
    }
}
