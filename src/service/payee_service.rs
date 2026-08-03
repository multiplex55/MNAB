use super::account_service::Ledger;
use crate::domain::*;

pub struct PayeeService<'a> {
    ledger: &'a mut Ledger,
}
impl<'a> PayeeService<'a> {
    pub fn new(ledger: &'a mut Ledger) -> Self {
        Self { ledger }
    }
    pub fn create(&mut self, budget_id: BudgetId, name: impl Into<String>) -> Payee {
        let p = Payee::new(budget_id, name);
        self.ledger.payees.insert(p.id, p.clone());
        p
    }
    pub fn rename(&mut self, id: PayeeId, name: impl Into<String>) -> bool {
        self.ledger
            .payees
            .get_mut(&id)
            .map(|p| p.name = name.into())
            .is_some()
    }
    pub fn merge(&mut self, source: PayeeId, target: PayeeId) -> bool {
        if !self.ledger.payees.contains_key(&target) || source == target {
            return false;
        }
        let mut staged = self.ledger.clone();
        for t in staged
            .transactions
            .values_mut()
            .filter(|t| t.payee_id == Some(source))
        {
            t.payee_id = Some(target);
        }
        staged.payees.remove(&source);
        staged.audit.push("merge payees".into());
        *self.ledger = staged;
        true
    }
    pub fn hide_if_unused(&mut self, id: PayeeId) -> bool {
        if self
            .ledger
            .transactions
            .values()
            .any(|t| t.payee_id == Some(id))
        {
            return false;
        }
        self.ledger
            .payees
            .get_mut(&id)
            .map(|p| p.hidden = true)
            .is_some()
    }
    pub fn set_default_category(&mut self, id: PayeeId, category: Option<CategoryId>) -> bool {
        self.ledger
            .payees
            .get_mut(&id)
            .map(|p| p.default_category_id = category)
            .is_some()
    }
    pub fn record_category(&mut self, id: PayeeId, category: CategoryId) -> bool {
        self.ledger
            .payees
            .get_mut(&id)
            .map(|p| p.last_used_category_id = Some(category))
            .is_some()
    }
    pub fn suggestion(&self, id: PayeeId) -> Option<CategoryId> {
        let p = self.ledger.payees.get(&id)?;
        p.last_used_category_id.or(p.default_category_id)
    }
}

/// Stable transfer identity/presentation: identity is the account ID; name is resolved at render time.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TransferPayee(pub AccountId);
impl TransferPayee {
    pub fn display(self, ledger: &Ledger) -> Option<String> {
        ledger
            .accounts
            .get(&self.0)
            .map(|a| format!("Transfer: {}", a.name))
    }
}
