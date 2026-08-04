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
unsupported!(TargetRepository,put_target:Target);
unsupported!(ScheduledRepository,put_scheduled:ScheduledTransaction);
unsupported!(ImportRepository,put_import_batch:ImportBatch);
unsupported!(ReconciliationRepository,put_reconciliation:Reconciliation);
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
