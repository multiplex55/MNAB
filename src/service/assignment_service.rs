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
