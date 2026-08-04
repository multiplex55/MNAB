//! Read-only, bounded projection queries.
//!
//! Register paging is keyset based.  The key is deliberately the persisted
//! `(transaction_date, id)` pair: dates are not unique and an offset alone can
//! skip or repeat rows when a transaction is inserted while the user scrolls.
use crate::{
    domain::{AccountId, BudgetId, Money},
    error::RepositoryError,
};
use rusqlite::{Connection, params_from_iter, types::Value};

pub const MAX_REGISTER_PAGE_SIZE: usize = 200;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RegisterRow {
    pub transaction_id: String,
    pub account_id: String,
    pub account_name: String,
    pub date: String,
    pub payee: String,
    pub category: String,
    pub memo: Option<String>,
    pub amount: Money,
    pub running_balance: Money,
    pub cleared_state: String,
    pub approval_state: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RegisterCursor {
    pub transaction_date: String,
    pub transaction_id: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RegisterFilter {
    /// Space separated words; quoted phrases and `payee:`, `category:`,
    /// `account:` and `memo:` fields are supported.
    pub search: String,
    pub from: Option<String>,
    pub through: Option<String>,
    pub category_ids: Vec<String>,
    pub payee_ids: Vec<String>,
    pub cleared_state: Option<String>,
    pub approval_state: Option<String>,
    pub minimum_amount: Option<Money>,
    pub maximum_amount: Option<Money>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RegisterScope {
    Account(AccountId),
    AllAccounts(BudgetId),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReconciliationSeparator {
    pub reconciliation_id: String,
    pub account_id: String,
    pub statement_date: String,
    pub ending_balance: Money,
    pub state: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RegisterPage {
    pub rows: Vec<RegisterRow>,
    pub next_cursor: Option<RegisterCursor>,
    /// Balance immediately before the oldest row in this page, per account.
    pub running_balance_anchors: Vec<(String, Money)>,
    pub reconciliation_separators: Vec<ReconciliationSeparator>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BudgetRow {
    pub category_id: String,
    pub assigned: Money,
}

pub struct QueryStore<'a> {
    connection: &'a Connection,
}
impl<'a> QueryStore<'a> {
    #[must_use]
    pub const fn new(connection: &'a Connection) -> Self {
        Self { connection }
    }

    /// Compatibility helper. Unlike the old unbounded query this can return at
    /// most [`MAX_REGISTER_PAGE_SIZE`] rows.
    pub fn register(&self, account: AccountId) -> Result<Vec<RegisterRow>, RepositoryError> {
        Ok(self
            .register_page(
                RegisterScope::Account(account),
                &RegisterFilter::default(),
                None,
                MAX_REGISTER_PAGE_SIZE,
            )?
            .rows)
    }

    pub fn register_page(
        &self,
        scope: RegisterScope,
        filter: &RegisterFilter,
        after: Option<&RegisterCursor>,
        requested_size: usize,
    ) -> Result<RegisterPage, RepositoryError> {
        let limit = requested_size.clamp(1, MAX_REGISTER_PAGE_SIZE);
        let mut where_sql = vec!["t.archived=0".to_owned()];
        let mut values = Vec::<Value>::new();
        match scope {
            RegisterScope::Account(id) => push(
                &mut where_sql,
                &mut values,
                "t.account_id=?",
                id.to_string(),
            ),
            RegisterScope::AllAccounts(id) => {
                push(&mut where_sql, &mut values, "t.budget_id=?", id.to_string())
            }
        }
        if let Some(from) = &filter.from {
            push(
                &mut where_sql,
                &mut values,
                "t.transaction_date>=?",
                from.clone(),
            );
        }
        if let Some(through) = &filter.through {
            push(
                &mut where_sql,
                &mut values,
                "t.transaction_date<=?",
                through.clone(),
            );
        }
        if let Some(state) = &filter.cleared_state {
            push(
                &mut where_sql,
                &mut values,
                "t.cleared_state=?",
                state.clone(),
            );
        }
        if let Some(state) = &filter.approval_state {
            push(
                &mut where_sql,
                &mut values,
                "t.approval_state=?",
                state.clone(),
            );
        }
        if let Some(amount) = filter.minimum_amount {
            push_value(
                &mut where_sql,
                &mut values,
                "t.amount>=?",
                amount.minor_units(),
            );
        }
        if let Some(amount) = filter.maximum_amount {
            push_value(
                &mut where_sql,
                &mut values,
                "t.amount<=?",
                amount.minor_units(),
            );
        }
        push_in(
            &mut where_sql,
            &mut values,
            "t.category_id",
            &filter.category_ids,
        );
        push_in(&mut where_sql, &mut values, "t.payee_id", &filter.payee_ids);
        for term in search_terms(&filter.search) {
            let (column, needle) = match term.field.as_deref() {
                Some("payee") => ("COALESCE(p.name,t.payee_snapshot,'')", term.text),
                Some("category") => ("COALESCE(c.name,'')", term.text),
                Some("account") => ("a.name", term.text),
                Some("memo") => ("COALESCE(t.memo,'')", term.text),
                _ => (
                    "(COALESCE(p.name,t.payee_snapshot,'')||' '||COALESCE(c.name,'')||' '||COALESCE(t.memo,'')||' '||a.name)",
                    term.text,
                ),
            };
            push(
                &mut where_sql,
                &mut values,
                &format!("LOWER({column}) LIKE ? ESCAPE '\\'"),
                format!("%{}%", escape_like(&needle.to_lowercase())),
            );
        }
        if let Some(cursor) = after {
            where_sql
                .push("(t.transaction_date < ? OR (t.transaction_date = ? AND t.id < ?))".into());
            values.extend([
                cursor.transaction_date.clone().into(),
                cursor.transaction_date.clone().into(),
                cursor.transaction_id.clone().into(),
            ]);
        }
        values.push((limit as i64 + 1).into());
        let sql = format!(
            r#"
            SELECT t.id,t.account_id,a.name,t.transaction_date,
                   COALESCE(p.name,t.payee_snapshot,''),COALESCE(c.name,''),t.memo,t.amount,
                   COALESCE((SELECT SUM(rb.amount) FROM transactions rb
                     WHERE rb.account_id=t.account_id AND rb.archived=0
                     AND (rb.transaction_date<t.transaction_date OR
                          (rb.transaction_date=t.transaction_date AND rb.id<=t.id))),0),
                   t.cleared_state,t.approval_state
            FROM transactions t JOIN accounts a ON a.id=t.account_id
            LEFT JOIN payees p ON p.id=t.payee_id LEFT JOIN categories c ON c.id=t.category_id
            WHERE {} ORDER BY t.transaction_date DESC,t.id DESC LIMIT ?"#,
            where_sql.join(" AND ")
        );
        let mut statement = self.connection.prepare(&sql).map_err(repo)?;
        let mut rows = statement
            .query_map(params_from_iter(values), |r| {
                Ok(RegisterRow {
                    transaction_id: r.get(0)?,
                    account_id: r.get(1)?,
                    account_name: r.get(2)?,
                    date: r.get(3)?,
                    payee: r.get(4)?,
                    category: r.get(5)?,
                    memo: r.get(6)?,
                    amount: Money::from_minor_units(r.get(7)?),
                    running_balance: Money::from_minor_units(r.get(8)?),
                    cleared_state: r.get(9)?,
                    approval_state: r.get(10)?,
                })
            })
            .map_err(repo)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(repo)?;
        let has_more = rows.len() > limit;
        rows.truncate(limit);
        let next_cursor = has_more
            .then(|| {
                rows.last().map(|r| RegisterCursor {
                    transaction_date: r.date.clone(),
                    transaction_id: r.transaction_id.clone(),
                })
            })
            .flatten();
        let mut anchors = Vec::new();
        for row in &rows {
            if !anchors.iter().any(|(id, _)| id == &row.account_id) {
                let oldest = rows
                    .iter()
                    .rev()
                    .find(|candidate| candidate.account_id == row.account_id)
                    .unwrap();
                anchors.push((
                    row.account_id.clone(),
                    oldest
                        .running_balance
                        .checked_sub(oldest.amount)
                        .map_err(|e| repo(e))?,
                ));
            }
        }
        let separators = self.separators(
            &scope,
            rows.last().map(|r| r.date.as_str()),
            rows.first().map(|r| r.date.as_str()),
        )?;
        Ok(RegisterPage {
            rows,
            next_cursor,
            running_balance_anchors: anchors,
            reconciliation_separators: separators,
        })
    }

    fn separators(
        &self,
        scope: &RegisterScope,
        from: Option<&str>,
        through: Option<&str>,
    ) -> Result<Vec<ReconciliationSeparator>, RepositoryError> {
        let (Some(from), Some(through)) = (from, through) else {
            return Ok(vec![]);
        };
        let (clause, owner) = match scope {
            RegisterScope::Account(id) => ("r.account_id", id.to_string()),
            RegisterScope::AllAccounts(id) => ("r.budget_id", id.to_string()),
        };
        let sql = format!(
            "SELECT r.id,r.account_id,r.statement_date,r.ending_balance,r.state FROM reconciliations r WHERE {clause}=?1 AND r.statement_date>=?2 AND r.statement_date<=?3 ORDER BY r.statement_date DESC,r.id DESC"
        );
        let mut stmt = self.connection.prepare(&sql).map_err(repo)?;
        stmt.query_map((owner, from, through), |r| {
            Ok(ReconciliationSeparator {
                reconciliation_id: r.get(0)?,
                account_id: r.get(1)?,
                statement_date: r.get(2)?,
                ending_balance: Money::from_minor_units(r.get(3)?),
                state: r.get(4)?,
            })
        })
        .map_err(repo)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(repo)
    }

    pub fn budget_month(
        &self,
        budget: BudgetId,
        month: &str,
    ) -> Result<Vec<BudgetRow>, RepositoryError> {
        let mut statement=self.connection.prepare("SELECT category_id,amount FROM budget_assignments WHERE budget_id=?1 AND budget_month=?2 ORDER BY category_id").map_err(repo)?;
        statement
            .query_map((budget.to_string(), month), |r| {
                Ok(BudgetRow {
                    category_id: r.get(0)?,
                    assigned: Money::from_minor_units(r.get(1)?),
                })
            })
            .map_err(repo)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(repo)
    }
}

fn push(where_sql: &mut Vec<String>, values: &mut Vec<Value>, clause: &str, value: String) {
    where_sql.push(clause.replace('?', &format!("?{}", values.len() + 1)));
    values.push(value.into());
}
fn push_value(where_sql: &mut Vec<String>, values: &mut Vec<Value>, clause: &str, value: i64) {
    where_sql.push(clause.replace('?', &format!("?{}", values.len() + 1)));
    values.push(value.into());
}
fn push_in(where_sql: &mut Vec<String>, values: &mut Vec<Value>, column: &str, items: &[String]) {
    if !items.is_empty() {
        let marks = (0..items.len())
            .map(|_| {
                values.push(Value::Null);
                format!("?{}", values.len())
            })
            .collect::<Vec<_>>();
        for (slot, item) in values.iter_mut().rev().take(items.len()).rev().zip(items) {
            *slot = item.clone().into();
        }
        where_sql.push(format!("{column} IN ({})", marks.join(",")));
    }
}
#[derive(Debug)]
struct SearchTerm {
    field: Option<String>,
    text: String,
}
fn search_terms(input: &str) -> Vec<SearchTerm> {
    let mut out = vec![];
    let mut chars = input.chars().peekable();
    while chars.peek().is_some() {
        while chars.peek().is_some_and(|c| c.is_whitespace()) {
            chars.next();
        }
        let mut token = String::new();
        let mut quoted = false;
        for c in chars.by_ref() {
            if c == '\"' {
                quoted = !quoted;
                continue;
            }
            if c.is_whitespace() && !quoted {
                break;
            }
            token.push(c);
        }
        if !token.is_empty() {
            let (field, text) = token
                .split_once(':')
                .map_or((None, token.clone()), |(f, t)| {
                    (
                        matches!(f, "payee" | "category" | "account" | "memo")
                            .then(|| f.to_owned()),
                        if matches!(f, "payee" | "category" | "account" | "memo") {
                            t.to_owned()
                        } else {
                            token.clone()
                        },
                    )
                });
            out.push(SearchTerm { field, text });
        }
    }
    out
}
fn escape_like(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}
fn repo<E: std::error::Error + Send + Sync + 'static>(source: E) -> RepositoryError {
    RepositoryError::Failed {
        source: Box::new(source),
    }
}
