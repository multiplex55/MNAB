//! SQLite mutation repositories. Every handle borrows one transaction and cannot outlive it.
use super::{mapping::validate_transaction, repository::*};
use crate::{domain::*, error::RepositoryError};
use rusqlite::Transaction as SqlTransaction;

pub struct SqliteRepositories<'tx> {
    pub(crate) transaction: SqlTransaction<'tx>,
}
impl<'tx> SqliteRepositories<'tx> {
    pub(crate) const fn new(transaction: SqlTransaction<'tx>) -> Self {
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
    fn account(&mut self, id: AccountId) -> Result<Option<Account>, RepositoryError> {
        use rusqlite::OptionalExtension;
        self.transaction.query_row("SELECT budget_id,name,account_type,closed,note,sort_order,favorite FROM accounts WHERE id=?1",[id.to_string()],|r|Ok(Account{id,budget_id:parse(r.get::<_,String>(0)?)?,group_id:None,name:r.get(1)?,account_type:parse_account_type(&r.get::<_,String>(2)?)?,closed:r.get(3)?,note:r.get(4)?,sort_order:r.get(5)?,favorite:r.get(6)?})).optional().map_err(repo)
    }
}
fn parse<T: std::str::FromStr>(s: String) -> rusqlite::Result<T> {
    s.parse().map_err(|_| rusqlite::Error::InvalidQuery)
}
fn parse_account_type(s: &str) -> rusqlite::Result<AccountType> {
    match s {
        "checking" => Ok(AccountType::Checking),
        "savings" => Ok(AccountType::Savings),
        "cash" => Ok(AccountType::Cash),
        "credit_card" => Ok(AccountType::CreditCard),
        "loan" => Ok(AccountType::Loan),
        "asset" => Ok(AccountType::Asset),
        "liability" => Ok(AccountType::Liability),
        "investment" => Err(rusqlite::Error::InvalidQuery),
        _ => Err(rusqlite::Error::InvalidQuery),
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
    }
}
impl TransactionRepository for SqliteRepositories<'_> {
    fn put_transaction(&mut self, v: &crate::domain::Transaction) -> Result<(), RepositoryError> {
        validate_transaction(v).map_err(repo)?;
        let (category, transfer) = match &v.body {
            TransactionBody::OpeningBalance { category_id } => (*category_id, None),
            TransactionBody::Categorized { category_id } => (Some(*category_id), None),
            TransactionBody::Transfer { transfer_id, .. } => (None, Some(*transfer_id)),
            TransactionBody::Split { .. } => (None, None),
        };
        self.transaction.execute("INSERT INTO transactions(id,budget_id,account_id,transaction_date,payee_id,category_id,transfer_id,amount,memo,cleared_state,approval_state,created_at,modified_at,archived,voided) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,datetime('now'),datetime('now'),?12,?13) ON CONFLICT(id) DO UPDATE SET account_id=excluded.account_id,transaction_date=excluded.transaction_date,payee_id=excluded.payee_id,category_id=excluded.category_id,transfer_id=excluded.transfer_id,amount=excluded.amount,memo=excluded.memo,cleared_state=excluded.cleared_state,approval_state=excluded.approval_state,modified_at=datetime('now'),archived=excluded.archived,voided=excluded.voided",(v.id.to_string(),v.budget_id.to_string(),v.account_id.to_string(),v.date.0.to_string(),v.payee_id.map(|x|x.to_string()),category.map(|x|x.to_string()),transfer.map(|x|x.to_string()),v.amount.minor_units(),&v.memo,match v.clearance{Clearance::Uncleared=>"uncleared",Clearance::Cleared=>"cleared",Clearance::Reconciled=>"reconciled"},match v.approval{Approval::Unapproved=>"unapproved",Approval::Approved=>"approved"},v.archived,v.voided)).map_err(repo)?;
        self.transaction
            .execute(
                "DELETE FROM subtransactions WHERE transaction_id=?1",
                [v.id.to_string()],
            )
            .map_err(repo)?;
        if let TransactionBody::Split { lines } = &v.body {
            for (i, line) in lines.iter().enumerate() {
                self.transaction.execute("INSERT INTO subtransactions(id,budget_id,transaction_id,category_id,memo,amount,sort_order) VALUES(?1,?2,?3,?4,?5,?6,?7)",(uuid::Uuid::new_v4().to_string(),v.budget_id.to_string(),v.id.to_string(),line.category_id.to_string(),&line.memo,line.amount.minor_units(),i as i64)).map_err(repo)?;
            }
        }
        Ok(())
    }
    fn transaction(
        &mut self,
        id: TransactionId,
    ) -> Result<Option<crate::domain::Transaction>, RepositoryError> {
        use rusqlite::OptionalExtension;
        let raw=self.transaction.query_row("SELECT budget_id,account_id,transaction_date,payee_id,category_id,transfer_id,amount,memo,cleared_state,approval_state,archived,voided FROM transactions WHERE id=?1",[id.to_string()],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,Option<String>>(3)?,r.get::<_,Option<String>>(4)?,r.get::<_,Option<String>>(5)?,r.get::<_,i64>(6)?,r.get::<_,Option<String>>(7)?,r.get::<_,String>(8)?,r.get::<_,String>(9)?,r.get::<_,bool>(10)?,r.get::<_,bool>(11)?))).optional().map_err(repo)?;
        let Some((b, a, d, p, c, t, amount, memo, clear, approval, archived, voided)) = raw else {
            return Ok(None);
        };
        let account_id: AccountId = a.parse().map_err(repo)?;
        let category = c.map(|x| x.parse()).transpose().map_err(repo)?;
        let split_lines = self.transaction.prepare("SELECT category_id,amount,memo FROM subtransactions WHERE transaction_id=?1 ORDER BY sort_order").map_err(repo)?.query_map([id.to_string()], |r| Ok(Subtransaction { category_id: parse(r.get(0)?)?, amount:Money::from_minor_units(r.get(1)?), memo:r.get(2)? })).map_err(repo)?.collect::<Result<Vec<_>,_>>().map_err(repo)?;
        let body = if !split_lines.is_empty() {
            TransactionBody::Split { lines: split_lines }
        } else if let Some(transfer) = t {
            TransactionBody::Transfer {
                transfer_id: transfer.parse().map_err(repo)?,
                source_account_id: account_id,
                destination_account_id: account_id,
                amount: Money::from_minor_units(amount),
                other_account_id: account_id,
                other_amount: Money::from_minor_units(-amount),
                category_id: category,
                category_effect_account_id: category.map(|_| account_id),
            }
        } else if let Some(category_id) = category {
            TransactionBody::Categorized { category_id }
        } else {
            TransactionBody::OpeningBalance { category_id: None }
        };
        Ok(Some(crate::domain::Transaction {
            id,
            budget_id: b.parse().map_err(repo)?,
            account_id,
            date: TransactionDate(
                time::Date::parse(&d, &time::format_description::well_known::Iso8601::DATE)
                    .map_err(repo)?,
            ),
            payee_id: p.map(|x| x.parse()).transpose().map_err(repo)?,
            amount: Money::from_minor_units(amount),
            memo,
            clearance: match clear.as_str() {
                "cleared" => Clearance::Cleared,
                "reconciled" => Clearance::Reconciled,
                _ => Clearance::Uncleared,
            },
            approval: if approval == "approved" {
                Approval::Approved
            } else {
                Approval::Unapproved
            },
            body,
            archived,
            voided,
        }))
    }
    fn delete_transaction(&mut self, id: TransactionId) -> Result<(), RepositoryError> {
        self.transaction
            .execute("DELETE FROM transactions WHERE id=?1", [id.to_string()])
            .map(|_| ())
            .map_err(repo)
    }
    fn selected_transactions(
        &mut self,
        selection: &crate::app::command::TransactionBatchSelection,
        limit: usize,
    ) -> Result<Vec<Transaction>, RepositoryError> {
        use crate::app::command::TransactionBatchSelection;
        let ids: Vec<TransactionId> = match selection {
            TransactionBatchSelection::Explicit(ids) => ids.iter().copied().collect(),
            TransactionBatchSelection::AllMatching { query, exclusions } => {
                // Fetch identities in bounded pages. Filtering domain values here keeps SQL an
                // implementation detail and, importantly, happens under the same transaction as writes.
                let mut statement = self
                    .transaction
                    .prepare("SELECT id FROM transactions ORDER BY transaction_date,id LIMIT ?1")
                    .map_err(repo)?;
                let raw = statement
                    .query_map([i64::try_from(limit + 1).unwrap_or(i64::MAX)], |r| {
                        r.get::<_, String>(0)
                    })
                    .map_err(repo)?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(repo)?;
                drop(statement);
                let mut matched = Vec::new();
                for raw_id in raw {
                    let id = raw_id.parse().map_err(repo)?;
                    if exclusions.contains(&id) {
                        continue;
                    }
                    if let Some(t) = self.transaction(id)? {
                        if sqlite_transaction_matches(&t, query) {
                            matched.push(id);
                        }
                    }
                }
                matched
            }
        };
        if ids.len() > limit {
            return Err(repo(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "batch selection exceeds safety limit",
            )));
        }
        let mut rows = Vec::new();
        for id in ids {
            if let Some(row) = self.transaction(id)? {
                rows.push(row);
            }
        }
        Ok(rows)
    }
}

fn sqlite_transaction_matches(t: &Transaction, q: &crate::app::register::CanonicalQuery) -> bool {
    use crate::app::view_model::RegisterScope;
    let f = &q.filter;
    (matches!(q.scope, RegisterScope::AllTransactions)
        || matches!(q.scope, RegisterScope::Account(id) if id==t.account_id))
        && f.from.is_none_or(|d| t.date.0 >= d)
        && f.through.is_none_or(|d| t.date.0 <= d)
        && f.minimum_amount_cents
            .is_none_or(|n| t.amount.minor_units() >= n)
        && f.maximum_amount_cents
            .is_none_or(|n| t.amount.minor_units() <= n)
        && (f.payee_ids.is_empty() || t.payee_id.is_some_and(|id| f.payee_ids.contains(&id)))
        && f.cleared_state.as_deref().is_none_or(|s| {
            s == match t.clearance {
                Clearance::Uncleared => "uncleared",
                Clearance::Cleared => "cleared",
                Clearance::Reconciled => "reconciled",
            }
        })
        && f.approval_state.as_deref().is_none_or(|s| {
            s == match t.approval {
                Approval::Approved => "approved",
                Approval::Unapproved => "unapproved",
            }
        })
        && (f.search.is_empty()
            || t.memo
                .as_deref()
                .unwrap_or("")
                .to_lowercase()
                .contains(&f.search.to_lowercase()))
}
macro_rules! unsupported {($trait:ident,$($method:ident:$ty:ty),+) => {impl $trait for SqliteRepositories<'_>{$ (fn $method(&mut self,_:&$ty)->Result<(),RepositoryError>{Err(repo(std::io::Error::new(std::io::ErrorKind::Unsupported,"repository operation is not implemented")))})+}}}
impl PayeeRepository for SqliteRepositories<'_> {
    fn put_payee(&mut self, v: &Payee) -> Result<(), RepositoryError> {
        self.transaction.execute("INSERT INTO payees(id,budget_id,name,archived,hidden,default_category_id,last_used_category_id) VALUES(?1,?2,?3,0,?4,?5,?6) ON CONFLICT(id) DO UPDATE SET name=excluded.name,hidden=excluded.hidden,default_category_id=excluded.default_category_id,last_used_category_id=excluded.last_used_category_id",(v.id.to_string(),v.budget_id.to_string(),&v.name,v.hidden,v.default_category_id.map(|x|x.to_string()),v.last_used_category_id.map(|x|x.to_string()))).map(|_|()).map_err(repo)
    }
    fn payee(&mut self, id: PayeeId) -> Result<Option<Payee>, RepositoryError> {
        use rusqlite::OptionalExtension;
        self.transaction.query_row("SELECT budget_id,name,hidden,default_category_id,last_used_category_id FROM payees WHERE id=?1",[id.to_string()],|r|Ok(Payee{id,budget_id:parse(r.get(0)?)?,name:r.get(1)?,hidden:r.get(2)?,default_category_id:r.get::<_,Option<String>>(3)?.map(|x|x.parse()).transpose().map_err(|_|rusqlite::Error::InvalidQuery)?,last_used_category_id:r.get::<_,Option<String>>(4)?.map(|x|x.parse()).transpose().map_err(|_|rusqlite::Error::InvalidQuery)?})).optional().map_err(repo)
    }
    fn payee_is_used(&mut self, id: PayeeId) -> Result<bool, RepositoryError> {
        self.transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM transactions WHERE payee_id=?1)",
                [id.to_string()],
                |r| r.get(0),
            )
            .map_err(repo)
    }
    fn delete_payee(&mut self, id: PayeeId) -> Result<(), RepositoryError> {
        self.transaction
            .execute("DELETE FROM payees WHERE id=?1", [id.to_string()])
            .map(|_| ())
            .map_err(repo)
    }
}
impl AssignmentRepository for SqliteRepositories<'_> {
    fn put_assignment(&mut self, v: &BudgetAssignment) -> Result<(), RepositoryError> {
        self.transaction.execute("INSERT INTO budget_assignments(id,budget_id,category_id,budget_month,amount,created_at,modified_at) SELECT ?1,c.budget_id,?2,?3,?4,datetime('now'),datetime('now') FROM categories c WHERE c.id=?2 ON CONFLICT(category_id,budget_month) DO UPDATE SET amount=excluded.amount,modified_at=datetime('now')",(uuid::Uuid::new_v4().to_string(),v.category_id.to_string(),month_text(v.month),v.amount.minor_units())).map(|_|()).map_err(repo)
    }
    fn assignment(
        &mut self,
        c: CategoryId,
        m: BudgetMonth,
    ) -> Result<Option<BudgetAssignment>, RepositoryError> {
        use rusqlite::OptionalExtension;
        self.transaction
            .query_row(
                "SELECT amount FROM budget_assignments WHERE category_id=?1 AND budget_month=?2",
                (c.to_string(), month_text(m)),
                |r| {
                    Ok(BudgetAssignment {
                        category_id: c,
                        month: m,
                        amount: Money::from_minor_units(r.get(0)?),
                    })
                },
            )
            .optional()
            .map_err(repo)
    }
    fn delete_assignment(&mut self, c: CategoryId, m: BudgetMonth) -> Result<(), RepositoryError> {
        self.transaction
            .execute(
                "DELETE FROM budget_assignments WHERE category_id=?1 AND budget_month=?2",
                (c.to_string(), month_text(m)),
            )
            .map(|_| ())
            .map_err(repo)
    }
    fn assignment_revision(&mut self, c: CategoryId) -> Result<u64, RepositoryError> {
        self.transaction.query_row(
            "SELECT COALESCE(MAX(l.id),0) FROM categories c JOIN category_groups g ON g.id=c.group_id LEFT JOIN change_log l ON l.budget_id=g.budget_id WHERE c.id=?1",
            [c.to_string()], |row| row.get(0),
        ).map_err(repo)
    }
}
fn month_text(m: BudgetMonth) -> String {
    format!("{:04}-{:02}", m.year(), m.month())
}
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
    fn delete_target(&mut self, id: TargetId) -> Result<(), RepositoryError> {
        self.transaction
            .execute("DELETE FROM targets WHERE id=?1", [id.to_string()])
            .map(|_| ())
            .map_err(repo)
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
        self.transaction.execute("INSERT INTO scheduled_transactions(id,budget_id,account_id,payee_id,category_id,start_date,recurrence,custom_interval_days,end_date,amount,sort_order,active,version) SELECT ?1,a.budget_id,?2,?3,?4,?5,?6,?7,?8,?9,0,?10,?11 FROM accounts a WHERE a.id=?2 ON CONFLICT(id) DO UPDATE SET account_id=excluded.account_id,payee_id=excluded.payee_id,category_id=excluded.category_id,start_date=excluded.start_date,recurrence=excluded.recurrence,custom_interval_days=excluded.custom_interval_days,end_date=excluded.end_date,amount=excluded.amount,active=excluded.active,version=excluded.version", (v.id.to_string(), v.account_id.to_string(), v.payee_id.map(|x| x.to_string()), v.category_id.map(|x| x.to_string()), v.start_date.to_string(), recurrence, interval, v.end_date.map(|x| x.to_string()), v.amount.minor_units(), v.active, v.version)).map(|_| ()).map_err(repo)
    }
    fn delete_scheduled(&mut self, id: ScheduledTransactionId) -> Result<(), RepositoryError> {
        self.transaction
            .execute(
                "DELETE FROM scheduled_transactions WHERE id=?1",
                [id.to_string()],
            )
            .map(|_| ())
            .map_err(repo)
    }
}
impl ReconciliationRepository for SqliteRepositories<'_> {
    fn put_reconciliation(&mut self, v: &Reconciliation) -> Result<(), RepositoryError> {
        let state = match v.state {
            ReconciliationState::NotReconciling => "not_reconciling",
            ReconciliationState::EnteringStatement => "entering_statement",
            ReconciliationState::Active => "active",
            ReconciliationState::ReviewingAdjustment => "reviewing_adjustment",
            ReconciliationState::Completing => "completing",
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
    fn put_group(&mut self, v: &CategoryGroup) -> Result<(), RepositoryError> {
        self.transaction.execute("INSERT INTO category_groups(id,budget_id,name,sort_order,hidden) VALUES(?1,?2,?3,COALESCE((SELECT MAX(sort_order)+1 FROM category_groups WHERE budget_id=?2),0),?4) ON CONFLICT(id) DO UPDATE SET name=excluded.name,hidden=excluded.hidden",(v.id.to_string(),v.budget_id.to_string(),&v.name,v.hidden)).map(|_|()).map_err(repo)
    }
    fn put_category(&mut self, v: &Category) -> Result<(), RepositoryError> {
        self.transaction.execute("INSERT INTO categories(id,budget_id,group_id,name,sort_order,hidden,archived) SELECT ?1,g.budget_id,?2,?3,COALESCE((SELECT MAX(sort_order)+1 FROM categories WHERE group_id=?2),0),?4,?5 FROM category_groups g WHERE g.id=?2 ON CONFLICT(id) DO UPDATE SET group_id=excluded.group_id,name=excluded.name,hidden=excluded.hidden,archived=excluded.archived",(v.id.to_string(),v.group_id.to_string(),&v.name,v.hidden,v.archived)).map(|_|()).map_err(repo)
    }
    fn category(&mut self, id: CategoryId) -> Result<Option<Category>, RepositoryError> {
        use rusqlite::OptionalExtension;
        self.transaction
            .query_row(
                "SELECT group_id,name,hidden,archived FROM categories WHERE id=?1",
                [id.to_string()],
                |r| {
                    Ok(Category {
                        id,
                        group_id: parse(r.get(0)?)?,
                        name: r.get(1)?,
                        hidden: r.get(2)?,
                        archived: r.get(3)?,
                    })
                },
            )
            .optional()
            .map_err(repo)
    }
    fn category_is_used(&mut self, id: CategoryId) -> Result<bool, RepositoryError> {
        self.transaction.query_row("SELECT EXISTS(SELECT 1 FROM transactions WHERE category_id=?1 UNION ALL SELECT 1 FROM subtransactions WHERE category_id=?1 UNION ALL SELECT 1 FROM budget_assignments WHERE category_id=?1)",[id.to_string()],|r|r.get(0)).map_err(repo)
    }
    fn category_is_managed(&mut self, id: CategoryId) -> Result<bool, RepositoryError> {
        self.transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM credit_card_payment_categories WHERE category_id=?1)",
                [id.to_string()],
                |r| r.get(0),
            )
            .map_err(repo)
    }
    fn delete_category(&mut self, id: CategoryId) -> Result<(), RepositoryError> {
        self.transaction
            .execute("DELETE FROM categories WHERE id=?1", [id.to_string()])
            .map(|_| ())
            .map_err(repo)
    }
}
impl AuditRepository for SqliteRepositories<'_> {
    fn append_audit(
        &mut self,
        entity: &str,
        record_id: &str,
        operation: &str,
    ) -> Result<(), RepositoryError> {
        let kind = if operation.starts_with("Create") || operation.starts_with("Save") {
            "insert"
        } else if operation.starts_with("Delete") {
            "delete"
        } else {
            "update"
        };
        let correlation = operation
            .rsplit_once("correlation=")
            .map_or("unknown", |(_, value)| value);
        // Assignment batch record ids are category ids, allowing the audit and projection
        // revision to advance atomically with the writes.
        let budget_id: Option<String> = if entity == "assignment_batch" {
            use rusqlite::OptionalExtension;
            self.transaction.query_row(
                "SELECT g.budget_id FROM categories c JOIN category_groups g ON g.id=c.group_id WHERE c.id=?1",
                [record_id], |row| row.get(0),
            ).optional().map_err(repo)?
        } else {
            None
        };
        self.transaction.execute("INSERT INTO change_log(budget_id,entity_table,entity_id,operation,changed_at,correlation_id) VALUES(?1,?2,?3,?4,datetime('now'),?5)",(budget_id,entity,record_id,kind,correlation)).map(|_|()).map_err(repo)
    }
}
impl InboxRepository for SqliteRepositories<'_> {
    fn toggle_failure_dismissal(&mut self, id: &str) -> Result<Option<bool>, RepositoryError> {
        use rusqlite::OptionalExtension;
        let before: Option<bool> = self
            .transaction
            .query_row(
                "SELECT dismissed_at IS NOT NULL FROM operation_failures WHERE id=?1",
                [id],
                |row| row.get(0),
            )
            .optional()
            .map_err(repo)?;
        if let Some(was_dismissed) = before {
            self.transaction.execute(
                "UPDATE operation_failures SET dismissed_at=CASE WHEN dismissed_at IS NULL THEN datetime('now') ELSE NULL END WHERE id=?1",
                [id],
            ).map_err(repo)?;
            Ok(Some(was_dismissed))
        } else {
            // A projected failure may disappear between scheduling and execution. Treat that as
            // an idempotent success rather than manufacturing a durable inbox row.
            Ok(None)
        }
    }
}
