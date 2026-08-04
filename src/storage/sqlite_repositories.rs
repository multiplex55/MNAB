//! SQLite mutation repositories. Every handle borrows one transaction and cannot outlive it.
use super::{mapping::validate_transaction, repository::*};
use crate::{domain::*, error::RepositoryError};
use rusqlite::Transaction;

pub struct SqliteRepositories<'tx> {
    pub(crate) transaction: Transaction<'tx>,
}
impl<'tx> SqliteRepositories<'tx> {
    pub(crate) const fn new(transaction: Transaction<'tx>) -> Self {
        Self { transaction }
    }
    pub(crate) fn commit(self) -> Result<(), RepositoryError> {
        self.transaction.commit().map_err(repo)
    }
    pub(crate) fn rollback(self) -> Result<(), RepositoryError> {
        self.transaction.rollback().map_err(repo)
    }
}
fn repo<E: std::error::Error + Send + Sync + 'static>(source: E) -> RepositoryError {
    RepositoryError::Failed {
        source: Box::new(source),
    }
}
impl BudgetRepository for SqliteRepositories<'_> {
    fn create_budget(&mut self, v: &Budget) -> Result<(), RepositoryError> {
        self.transaction.execute("INSERT INTO budgets(id,name,created_at,modified_at,archived) VALUES(?1,?2,datetime('now'),datetime('now'),0)",(v.id.to_string(),&v.name)).map(|_|()).map_err(repo)
    }
    fn budget(&mut self, id: BudgetId) -> Result<Option<Budget>, RepositoryError> {
        use rusqlite::OptionalExtension;
        let row = self
            .transaction
            .query_row(
                "SELECT id,name FROM budgets WHERE id=?1",
                [id.to_string()],
                |r| {
                    Ok(super::model::BudgetRow {
                        id: r.get(0)?,
                        name: r.get(1)?,
                    })
                },
            )
            .optional()
            .map_err(repo)?;
        row.map(TryInto::try_into).transpose().map_err(repo)
    }
}
impl AccountRepository for SqliteRepositories<'_> {
    fn put_account(&mut self, v: &Account) -> Result<(), RepositoryError> {
        self.transaction.execute("INSERT INTO accounts(id,budget_id,name,account_type,sort_order,closed,note,favorite,created_at,modified_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,datetime('now'),datetime('now')) ON CONFLICT(id) DO UPDATE SET name=excluded.name,sort_order=excluded.sort_order,closed=excluded.closed,note=excluded.note,favorite=excluded.favorite,modified_at=datetime('now')",(v.id.to_string(),v.budget_id.to_string(),&v.name,account_type(v.account_type),v.sort_order,v.closed,&v.note,v.favorite)).map(|_|()).map_err(repo)
    }
}
fn account_type(v: AccountType) -> &'static str {
    match v {
        AccountType::Checking => "checking",
        AccountType::Savings => "savings",
        AccountType::Cash => "cash",
        AccountType::CreditCard => "credit_card",
        AccountType::Loan => "loan",
        AccountType::Asset => "asset",
        AccountType::Liability => "liability",
        AccountType::Investment => "investment",
    }
}
impl TransactionRepository for SqliteRepositories<'_> {
    fn put_transaction(&mut self, v: &crate::domain::Transaction) -> Result<(), RepositoryError> {
        validate_transaction(v).map_err(repo)?;
        self.transaction.execute("INSERT INTO transactions(id,budget_id,account_id,transaction_date,payee_id,amount,memo,cleared_state,approval_state,created_at,modified_at,archived,voided) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,datetime('now'),datetime('now'),?10,?11)",(v.id.to_string(),v.budget_id.to_string(),v.account_id.to_string(),v.date.0.to_string(),v.payee_id.map(|x|x.to_string()),v.amount.minor_units(),&v.memo,match v.clearance{Clearance::Uncleared=>"uncleared",Clearance::Cleared=>"cleared",Clearance::Reconciled=>"reconciled"},match v.approval{Approval::Unapproved=>"unapproved",Approval::Approved=>"approved"},v.archived,v.voided)).map(|_|()).map_err(repo)
    }
}
macro_rules! unsupported {($trait:ident,$($method:ident:$ty:ty),+) => {impl $trait for SqliteRepositories<'_>{$ (fn $method(&mut self,_:&$ty)->Result<(),RepositoryError>{Err(repo(std::io::Error::new(std::io::ErrorKind::Unsupported,"repository operation is not implemented")))})+}}}
unsupported!(PayeeRepository,put_payee:Payee);
unsupported!(AssignmentRepository,put_assignment:BudgetAssignment);
unsupported!(ImportRepository,put_import_batch:ImportBatch);
impl TargetRepository for SqliteRepositories<'_> {
    fn put_target(&mut self, v: &Target) -> Result<(), RepositoryError> {
        let (category, account) = match v.association {
            TargetAssociation::Category(id) => (id, None),
            TargetAssociation::CreditCard {
                account_id,
                payment_category_id,
            } => (payment_category_id, Some(account_id)),
        };
        let (kind, amount, due, recurrence) = match v.kind {
            TargetKind::BalanceAmount { amount } => ("balance_amount", Some(amount), None, "none"),
            TargetKind::BalanceByDate { amount, due } => {
                ("balance_by_date", Some(amount), Some(due), "none")
            }
            TargetKind::FixedMonthlySavings { amount } => {
                ("fixed_monthly_savings", Some(amount), None, "none")
            }
            TargetKind::RefillToAmount { amount } => {
                ("refill_to_amount", Some(amount), None, "none")
            }
            TargetKind::UpcomingExpense {
                amount,
                due,
                recurrence,
            } => (
                "upcoming_expense",
                Some(amount),
                Some(due),
                match recurrence {
                    TargetRecurrence::None => "none",
                    TargetRecurrence::Monthly => "monthly",
                    TargetRecurrence::Yearly => "yearly",
                },
            ),
            TargetKind::CreditCardPayoffByDate { due } => {
                ("credit_card_payoff_by_date", None, Some(due), "none")
            }
        };
        self.transaction.execute("INSERT INTO targets(id,budget_id,category_id,account_id,target_type,amount,due_date,recurrence,created_at,modified_at) SELECT ?1,c.budget_id,?2,?3,?4,?5,?6,?7,datetime('now'),datetime('now') FROM categories c WHERE c.id=?2 ON CONFLICT(id) DO UPDATE SET category_id=excluded.category_id,account_id=excluded.account_id,target_type=excluded.target_type,amount=excluded.amount,due_date=excluded.due_date,recurrence=excluded.recurrence,modified_at=datetime('now')", (v.id.to_string(), category.to_string(), account.map(|x| x.to_string()), kind, amount.map(Money::minor_units), due.map(|x| x.to_string()), recurrence)).map(|_| ()).map_err(repo)
    }
}
impl ScheduledRepository for SqliteRepositories<'_> {
    fn put_scheduled(&mut self, v: &ScheduledTransaction) -> Result<(), RepositoryError> {
        let (recurrence, interval) = match v.recurrence {
            Recurrence::Daily => ("daily", None),
            Recurrence::Weekly => ("weekly", None),
            Recurrence::EveryTwoWeeks => ("every_two_weeks", None),
            Recurrence::Monthly => ("monthly", None),
            Recurrence::Yearly => ("yearly", None),
            Recurrence::CustomDays(n) => ("custom_days", Some(n)),
        };
        self.transaction.execute("INSERT INTO scheduled_transactions(id,budget_id,account_id,payee_id,category_id,start_date,recurrence,custom_interval_days,end_date,amount,sort_order,active) SELECT ?1,a.budget_id,?2,?3,?4,?5,?6,?7,?8,?9,0,?10 FROM accounts a WHERE a.id=?2 ON CONFLICT(id) DO UPDATE SET account_id=excluded.account_id,payee_id=excluded.payee_id,category_id=excluded.category_id,start_date=excluded.start_date,recurrence=excluded.recurrence,custom_interval_days=excluded.custom_interval_days,end_date=excluded.end_date,amount=excluded.amount,active=excluded.active", (v.id.to_string(), v.account_id.to_string(), v.payee_id.map(|x| x.to_string()), v.category_id.map(|x| x.to_string()), v.start_date.to_string(), recurrence, interval, v.end_date.map(|x| x.to_string()), v.amount.minor_units(), v.active)).map(|_| ()).map_err(repo)
    }
}
impl ReconciliationRepository for SqliteRepositories<'_> {
    fn put_reconciliation(&mut self, v: &Reconciliation) -> Result<(), RepositoryError> {
        let state = match v.state {
            ReconciliationState::Active => "active",
            ReconciliationState::Completed => "completed",
            ReconciliationState::PotentiallyInvalid => "potentially_invalid",
        };
        self.transaction.execute("INSERT INTO reconciliations(id,budget_id,account_id,statement_date,ending_balance,calculated_cleared_balance,difference,state,created_at,completed_at,invalidated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11) ON CONFLICT(id) DO UPDATE SET calculated_cleared_balance=excluded.calculated_cleared_balance,difference=excluded.difference,state=excluded.state,completed_at=excluded.completed_at,invalidated_at=excluded.invalidated_at", (v.id.to_string(), v.budget_id.to_string(), v.account_id.to_string(), v.statement_date.0.to_string(), v.ending_balance.minor_units(), v.calculated_cleared_balance.minor_units(), v.difference.minor_units(), state, v.created_at.to_string(), v.completed_at.map(|x| x.to_string()), v.invalidated_at.map(|x| x.to_string()))).map_err(repo)?;
        for id in &v.included_transaction_ids {
            self.transaction.execute("INSERT OR IGNORE INTO reconciliation_transactions(reconciliation_id,budget_id,transaction_id,included_at) VALUES(?1,?2,?3,datetime('now'))", (v.id.to_string(), v.budget_id.to_string(), id.to_string())).map_err(repo)?;
            self.transaction.execute("UPDATE transactions SET cleared_state='reconciled',reconciliation_id=?1,modified_at=datetime('now') WHERE id=?2 AND budget_id=?3", (v.id.to_string(), id.to_string(), v.budget_id.to_string())).map_err(repo)?;
        }
        Ok(())
    }
}
impl CategoryRepository for SqliteRepositories<'_> {
    fn put_group(&mut self, _: &CategoryGroup) -> Result<(), RepositoryError> {
        Err(repo(std::io::Error::other("not implemented")))
    }
    fn put_category(&mut self, _: &Category) -> Result<(), RepositoryError> {
        Err(repo(std::io::Error::other("not implemented")))
    }
}
impl AuditRepository for SqliteRepositories<'_> {
    fn append_audit(
        &mut self,
        entity: &str,
        record_id: &str,
        operation: &str,
    ) -> Result<(), RepositoryError> {
        self.transaction.execute("INSERT INTO audit_log(entity_type,entity_id,operation,changed_at) VALUES(?1,?2,?3,datetime('now'))",(entity,record_id,operation)).map(|_|()).map_err(repo)
    }
}
