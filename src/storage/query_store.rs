//! Read-only, bounded projection queries.
//!
//! Register paging is keyset based.  The key is deliberately the persisted
//! `(transaction_date, id)` pair: dates are not unique and an offset alone can
//! skip or repeat rows when a transaction is inserted while the user scrolls.
use crate::{
    domain::{
        AccountId, AccountScope, BudgetId, BudgetMonth, IncomeExpenseResult, IncomeExpenseRow,
        Money, MonthlySpendingRow, PayeeSpendingRow, ReportFilter, ReportKind, ReportPresentation,
        ReportRequest, ReportResult, SourceData, SpendingResult, SpendingRow,
    },
    error::RepositoryError,
};
use rusqlite::{Connection, params_from_iter, types::Value};
use std::{collections::BTreeMap, str::FromStr};
use time::OffsetDateTime;

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

    /// Executes a report inside SQLite. Only typed aggregate rows cross this boundary; ledger
    /// rows remain owned by the storage thread.
    pub fn report(
        &self,
        budget: BudgetId,
        request: &ReportRequest,
    ) -> Result<ReportResult, RepositoryError> {
        match request.kind {
            ReportKind::Spending => self
                .spending_report(budget, &request.filter)
                .map(ReportResult::Spending),
            ReportKind::IncomeExpense => self
                .income_expense_report(budget, &request.filter)
                .map(ReportResult::IncomeExpense),
            ReportKind::NetWorth => self
                .net_worth_report(budget, &request.filter)
                .map(ReportResult::NetWorth),
            ReportKind::BudgetProgress => self
                .budget_progress_report(budget, &request.filter)
                .map(ReportResult::BudgetProgress),
        }
    }

    /// A report revision changes only when one of the tables capable of affecting that report is
    /// committed. This makes unrelated mutations retain their cache entries.
    pub fn report_revision(
        &self,
        budget: BudgetId,
        kind: ReportKind,
    ) -> Result<u64, RepositoryError> {
        let tables = match kind {
            ReportKind::Spending | ReportKind::IncomeExpense => {
                "'transactions','subtransactions','accounts','categories','category_groups','payees'"
            }
            ReportKind::NetWorth => "'transactions','accounts'",
            ReportKind::BudgetProgress => {
                "'transactions','subtransactions','accounts','categories','category_groups','budget_assignments','targets'"
            }
        };
        self.connection.query_row(&format!("SELECT COALESCE(MAX(id),0) FROM change_log WHERE budget_id=?1 AND entity_table IN ({tables})"), [budget.to_string()], |r| r.get::<_, i64>(0))
            .map(|v| v as u64).map_err(repo)
    }

    fn report_source(
        &self,
        budget: BudgetId,
        kind: ReportKind,
    ) -> Result<SourceData, RepositoryError> {
        Ok(SourceData {
            revision: self.report_revision(budget, kind)?,
            refreshed_at: OffsetDateTime::now_utc(),
        })
    }

    fn spending_report(
        &self,
        budget: BudgetId,
        filter: &ReportFilter,
    ) -> Result<SpendingResult, RepositoryError> {
        let (where_sql, values) = report_where(budget, filter, true, true);
        let cte = report_lines_cte();
        let sql = format!(
            "{cte} SELECT c.group_id,l.category_id,-SUM(l.amount) amount FROM lines l JOIN categories c ON c.id=l.category_id WHERE {where_sql} AND l.amount<0 GROUP BY c.group_id,l.category_id ORDER BY c.group_id,l.category_id"
        );
        let mut stmt = self.connection.prepare(&sql).map_err(repo)?;
        let rows = stmt
            .query_map(params_from_iter(values.clone()), |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                ))
            })
            .map_err(repo)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(repo)?
            .into_iter()
            .map(|(g, c, a)| {
                Ok(SpendingRow {
                    group_id: parse_id(&g)?,
                    category_id: parse_id(&c)?,
                    amount: Money::from_minor_units(a),
                })
            })
            .collect::<Result<Vec<_>, RepositoryError>>()?;
        let mut groups = BTreeMap::new();
        let mut total = Money::ZERO;
        for row in &rows {
            add_money(&mut groups, row.group_id, row.amount)?;
            total = total.checked_add(row.amount).map_err(repo)?;
        }
        let monthly = self
            .aggregate_spending_dimension::<String, _>(
                &cte,
                &where_sql,
                values.clone(),
                "substr(l.transaction_date,1,7)",
                |s| parse_month(&s),
            )?
            .into_iter()
            .map(|(month, amount)| MonthlySpendingRow { month, amount })
            .collect();
        let payees = self
            .aggregate_spending_dimension::<Option<String>, _>(
                &cte,
                &where_sql,
                values,
                "l.payee_id",
                |s| match s {
                    Some(v) => Ok(Some(parse_id(&v)?)),
                    None => Ok(None),
                },
            )?
            .into_iter()
            .map(|(payee_id, amount)| PayeeSpendingRow { payee_id, amount })
            .collect();
        let row_count = rows.len();
        Ok(SpendingResult {
            source: self.report_source(budget, ReportKind::Spending)?,
            rows,
            groups: groups.into_iter().collect(),
            monthly,
            payees,
            total,
            presentation: ReportPresentation {
                currency_code: "USD".into(),
                row_count,
                is_empty: row_count == 0,
            },
        })
    }

    fn aggregate_spending_dimension<T: rusqlite::types::FromSql, K: Ord>(
        &self,
        cte: &str,
        where_sql: &str,
        values: Vec<Value>,
        dimension: &str,
        convert: impl Fn(T) -> Result<K, RepositoryError>,
    ) -> Result<Vec<(K, Money)>, RepositoryError> {
        let sql = format!(
            "{cte} SELECT {dimension},-SUM(l.amount) FROM lines l JOIN categories c ON c.id=l.category_id WHERE {where_sql} AND l.amount<0 GROUP BY {dimension} ORDER BY {dimension}"
        );
        let mut stmt = self.connection.prepare(&sql).map_err(repo)?;
        stmt.query_map(params_from_iter(values), |r| {
            Ok((r.get::<_, T>(0)?, r.get::<_, i64>(1)?))
        })
        .map_err(repo)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(repo)?
        .into_iter()
        .map(|(key, value)| Ok((convert(key)?, Money::from_minor_units(value))))
        .collect()
    }

    fn income_expense_report(
        &self,
        budget: BudgetId,
        filter: &ReportFilter,
    ) -> Result<IncomeExpenseResult, RepositoryError> {
        let (where_sql, values) = report_where(budget, filter, true, true);
        let sql = format!(
            "{} SELECT substr(l.transaction_date,1,7),SUM(CASE WHEN l.amount>=0 THEN l.amount ELSE 0 END),-SUM(CASE WHEN l.amount<0 THEN l.amount ELSE 0 END) FROM lines l JOIN categories c ON c.id=l.category_id WHERE {where_sql} GROUP BY 1 ORDER BY 1",
            report_lines_cte()
        );
        let mut stmt = self.connection.prepare(&sql).map_err(repo)?;
        let raw = stmt
            .query_map(params_from_iter(values), |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, i64>(2)?,
                ))
            })
            .map_err(repo)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(repo)?;
        let mut rows = Vec::with_capacity(raw.len());
        let mut income = Money::ZERO;
        let mut expense = Money::ZERO;
        for (m, i, e) in raw {
            let i = Money::from_minor_units(i);
            let e = Money::from_minor_units(e);
            income = income.checked_add(i).map_err(repo)?;
            expense = expense.checked_add(e).map_err(repo)?;
            rows.push(IncomeExpenseRow {
                month: parse_month(&m)?,
                income: i,
                expense: e,
                net: i.checked_sub(e).map_err(repo)?,
            });
        }
        let net = income.checked_sub(expense).map_err(repo)?;
        let count = rows.len();
        Ok(IncomeExpenseResult {
            source: self.report_source(budget, ReportKind::IncomeExpense)?,
            rows,
            income,
            expense,
            net,
            presentation: presentation(count),
        })
    }

    fn net_worth_report(
        &self,
        budget: BudgetId,
        filter: &ReportFilter,
    ) -> Result<crate::domain::NetWorthResult, RepositoryError> {
        let (account_sql, account_values) = account_where(budget, filter);
        let mut stmt = self
            .connection
            .prepare(&format!(
                "SELECT a.id FROM accounts a WHERE {account_sql} ORDER BY a.id"
            ))
            .map_err(repo)?;
        let included_accounts = stmt
            .query_map(params_from_iter(account_values.clone()), |r| {
                r.get::<_, String>(0)
            })
            .map_err(repo)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(repo)?
            .into_iter()
            .map(|id| parse_id(&id))
            .collect::<Result<Vec<_>, _>>()?;
        let mut values = account_values;
        values.push(filter.dates.end.to_string().into());
        values.push(filter.dates.start.to_string().into());
        let sql = format!(
            r#"WITH selected AS (SELECT a.id,a.account_type FROM accounts a WHERE {account_sql}),
          dates AS (SELECT DISTINCT t.transaction_date d FROM transactions t JOIN selected a ON a.id=t.account_id WHERE t.archived=0 AND t.voided=0 AND t.transaction_date<=?),
          balances AS (SELECT d.d,a.account_type,COALESCE(SUM(t.amount),0) balance FROM dates d CROSS JOIN selected a LEFT JOIN transactions t ON t.account_id=a.id AND t.archived=0 AND t.voided=0 AND t.transaction_date<=d.d GROUP BY d.d,a.id)
          SELECT d,SUM(CASE WHEN account_type IN ('credit_card','loan','liability') THEN 0 ELSE balance END),SUM(CASE WHEN account_type IN ('credit_card','loan','liability') THEN balance ELSE 0 END) FROM balances WHERE d>=? GROUP BY d ORDER BY d"#
        );
        let mut stmt = self.connection.prepare(&sql).map_err(repo)?;
        let raw = stmt
            .query_map(params_from_iter(values), |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, i64>(2)?,
                ))
            })
            .map_err(repo)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(repo)?;
        let mut rows = Vec::with_capacity(raw.len());
        for (date, assets, liabilities) in raw {
            let assets = Money::from_minor_units(assets);
            let liabilities = Money::from_minor_units(liabilities);
            rows.push(crate::domain::NetWorthRow {
                date: time::Date::parse(
                    &date,
                    &time::format_description::well_known::Iso8601::DATE,
                )
                .map_err(repo)?,
                assets,
                liabilities,
                net_worth: assets.checked_add(liabilities).map_err(repo)?,
            });
        }
        let total = rows.last().map_or(Money::ZERO, |r| r.net_worth);
        let count = rows.len();
        Ok(crate::domain::NetWorthResult {
            source: self.report_source(budget, ReportKind::NetWorth)?,
            included_accounts,
            rows,
            total,
            presentation: presentation(count),
        })
    }

    fn budget_progress_report(
        &self,
        budget: BudgetId,
        filter: &ReportFilter,
    ) -> Result<crate::domain::BudgetProgressResult, RepositoryError> {
        let (where_sql, mut values) = report_where(budget, filter, true, false);
        // Assignment months are inclusive when their first day lies in the requested range.
        values.push(budget.to_string().into());
        values.push(filter.dates.start.to_string().into());
        values.push(filter.dates.end.to_string().into());
        let mut category_conditions = Vec::new();
        if !filter.category_ids.is_empty() {
            category_conditions.push(format!(
                "ba.category_id IN ({})",
                placeholders(filter.category_ids.len())
            ));
            values.extend(
                filter
                    .category_ids
                    .iter()
                    .map(|v| Value::from(v.to_string())),
            );
        }
        if !filter.category_group_ids.is_empty() {
            category_conditions.push(format!(
                "c.group_id IN ({})",
                placeholders(filter.category_group_ids.len())
            ));
            values.extend(
                filter
                    .category_group_ids
                    .iter()
                    .map(|v| Value::from(v.to_string())),
            );
        }
        let category_sql = if category_conditions.is_empty() {
            "1=1".into()
        } else {
            category_conditions.join(" AND ")
        };
        let sql = format!(
            r#"{} , spend AS (SELECT substr(l.transaction_date,1,7) month,l.category_id,-SUM(l.amount) spent FROM lines l JOIN categories c ON c.id=l.category_id WHERE {where_sql} AND l.amount<0 GROUP BY 1,2),
          assigned AS (SELECT ba.budget_month month,ba.category_id,SUM(ba.amount) assigned FROM budget_assignments ba JOIN categories c ON c.id=ba.category_id WHERE ba.budget_id=? AND ba.budget_month||'-01'>=? AND ba.budget_month||'-01'<=? AND {category_sql} GROUP BY 1,2),
          keys AS (SELECT month,category_id FROM spend UNION SELECT month,category_id FROM assigned)
          SELECT k.month,k.category_id,COALESCE(a.assigned,0),COALESCE(s.spent,0),CASE WHEN t.target_type='credit_card_payoff_by_date' THEN NULL ELSE t.amount END FROM keys k LEFT JOIN assigned a USING(month,category_id) LEFT JOIN spend s USING(month,category_id) LEFT JOIN targets t ON t.category_id=k.category_id ORDER BY k.month,k.category_id"#,
            report_lines_cte()
        );
        let mut stmt = self.connection.prepare(&sql).map_err(repo)?;
        let raw = stmt
            .query_map(params_from_iter(values), |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, i64>(3)?,
                    r.get::<_, Option<i64>>(4)?,
                ))
            })
            .map_err(repo)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(repo)?;
        let mut rows = Vec::with_capacity(raw.len());
        let mut total_assigned = Money::ZERO;
        let mut total_spent = Money::ZERO;
        for (month, category, assigned, spent, target) in raw {
            let assigned = Money::from_minor_units(assigned);
            let spent = Money::from_minor_units(spent);
            let target = target.map(Money::from_minor_units);
            total_assigned = total_assigned.checked_add(assigned).map_err(repo)?;
            total_spent = total_spent.checked_add(spent).map_err(repo)?;
            let available = assigned.checked_sub(spent).map_err(repo)?;
            rows.push(crate::domain::BudgetProgressRow {
                month: parse_month(&month)?,
                category_id: parse_id(&category)?,
                assigned,
                spent,
                target,
                target_completion_basis_points: target.map(|x| {
                    if x <= Money::ZERO {
                        10_000
                    } else {
                        ((assigned.max(Money::ZERO).minor_units() as i128 * 10_000
                            / i128::from(x.minor_units()))
                        .clamp(0, 10_000)) as u16
                    }
                }),
                underfunded: target.map_or(Money::ZERO, |x| {
                    x.checked_sub(assigned).unwrap().max(Money::ZERO)
                }),
                overspent: available
                    .checked_neg()
                    .unwrap_or(Money::ZERO)
                    .max(Money::ZERO),
            });
        }
        let count = rows.len();
        Ok(crate::domain::BudgetProgressResult {
            source: self.report_source(budget, ReportKind::BudgetProgress)?,
            rows,
            total_assigned,
            total_spent,
            presentation: presentation(count),
        })
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

fn report_lines_cte() -> String {
    r#"WITH lines AS (
      SELECT t.budget_id,t.account_id,t.transaction_date,t.payee_id,t.category_id,t.amount,t.archived,t.voided
      FROM transactions t WHERE NOT EXISTS (SELECT 1 FROM subtransactions s WHERE s.transaction_id=t.id)
      UNION ALL
      SELECT t.budget_id,t.account_id,t.transaction_date,t.payee_id,s.category_id,s.amount,t.archived,t.voided
      FROM transactions t JOIN subtransactions s ON s.transaction_id=t.id
    )"#.into()
}
fn placeholders(count: usize) -> String {
    std::iter::repeat_n("?", count)
        .collect::<Vec<_>>()
        .join(",")
}
fn account_where(budget: BudgetId, filter: &ReportFilter) -> (String, Vec<Value>) {
    let mut clauses = vec!["a.budget_id=?".into()];
    let mut values = vec![budget.to_string().into()];
    if !filter.account_ids.is_empty() {
        clauses.push(format!(
            "a.id IN ({})",
            placeholders(filter.account_ids.len())
        ));
        values.extend(
            filter
                .account_ids
                .iter()
                .map(|v| Value::from(v.to_string())),
        );
    }
    match filter.accounts {
        AccountScope::OnBudget => {
            clauses.push("a.account_type IN ('checking','savings','cash','credit_card')".into())
        }
        AccountScope::Tracking => {
            clauses.push("a.account_type IN ('loan','asset','liability','investment')".into())
        }
        AccountScope::Both => {}
    }
    (clauses.join(" AND "), values)
}
fn report_where(
    budget: BudgetId,
    filter: &ReportFilter,
    budget_only: bool,
    categories_required: bool,
) -> (String, Vec<Value>) {
    let mut clauses = vec![
        "l.budget_id=?".into(),
        "l.archived=0".into(),
        "l.voided=0".into(),
        "l.transaction_date>=?".into(),
        "l.transaction_date<=?".into(),
    ];
    let mut values = vec![
        budget.to_string().into(),
        filter.dates.start.to_string().into(),
        filter.dates.end.to_string().into(),
    ];
    if categories_required {
        clauses.push("l.category_id IS NOT NULL".into());
    }
    if !filter.account_ids.is_empty() {
        clauses.push(format!(
            "l.account_id IN ({})",
            placeholders(filter.account_ids.len())
        ));
        values.extend(
            filter
                .account_ids
                .iter()
                .map(|v| Value::from(v.to_string())),
        );
    }
    if !filter.category_ids.is_empty() {
        clauses.push(format!(
            "l.category_id IN ({})",
            placeholders(filter.category_ids.len())
        ));
        values.extend(
            filter
                .category_ids
                .iter()
                .map(|v| Value::from(v.to_string())),
        );
    }
    if !filter.category_group_ids.is_empty() {
        clauses.push(format!(
            "c.group_id IN ({})",
            placeholders(filter.category_group_ids.len())
        ));
        values.extend(
            filter
                .category_group_ids
                .iter()
                .map(|v| Value::from(v.to_string())),
        );
    }
    if !filter.payee_ids.is_empty() {
        clauses.push(format!(
            "l.payee_id IN ({})",
            placeholders(filter.payee_ids.len())
        ));
        values.extend(filter.payee_ids.iter().map(|v| Value::from(v.to_string())));
    }
    // Every report line query joins accounts under this alias.
    let classification = if budget_only || matches!(filter.accounts, AccountScope::OnBudget) {
        " AND a.account_type IN ('checking','savings','cash','credit_card')"
    } else if matches!(filter.accounts, AccountScope::Tracking) {
        " AND a.account_type IN ('loan','asset','liability','investment')"
    } else {
        ""
    };
    clauses.push(format!("EXISTS (SELECT 1 FROM accounts a WHERE a.id=l.account_id AND a.budget_id=l.budget_id{classification})"));
    (clauses.join(" AND "), values)
}
fn parse_id<T: FromStr>(value: &str) -> Result<T, RepositoryError>
where
    T::Err: std::error::Error + Send + Sync + 'static,
{
    value.parse().map_err(repo)
}
fn parse_month(value: &str) -> Result<BudgetMonth, RepositoryError> {
    let (year, month) = value.split_once('-').ok_or_else(|| {
        repo(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "invalid month",
        ))
    })?;
    BudgetMonth::new(year.parse().map_err(repo)?, month.parse().map_err(repo)?).map_err(repo)
}
fn add_money<K: Ord + Copy>(
    map: &mut BTreeMap<K, Money>,
    key: K,
    value: Money,
) -> Result<(), RepositoryError> {
    let next = map
        .get(&key)
        .copied()
        .unwrap_or(Money::ZERO)
        .checked_add(value)
        .map_err(repo)?;
    map.insert(key, next);
    Ok(())
}
fn presentation(row_count: usize) -> ReportPresentation {
    ReportPresentation {
        currency_code: "USD".into(),
        row_count,
        is_empty: row_count == 0,
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

#[cfg(test)]
mod report_tests {
    use super::*;
    use crate::domain::{DateRange, ReportFilter};
    use std::collections::BTreeSet;
    use time::macros::date;

    #[test]
    fn every_aggregate_query_returns_a_bounded_typed_empty_result() {
        let directory = tempfile::tempdir().unwrap();
        let connection =
            crate::storage::connection::open_primary(&directory.path().join("reports.sqlite3"))
                .unwrap();
        let budget = BudgetId::new();
        connection.execute("INSERT INTO budgets(id,name,created_at,modified_at,archived) VALUES(?1,'Reports',datetime('now'),datetime('now'),0)",[budget.to_string()]).unwrap();
        let filter = ReportFilter {
            dates: DateRange {
                start: date!(2026 - 01 - 01),
                end: date!(2026 - 12 - 31),
            },
            account_ids: BTreeSet::new(),
            category_group_ids: BTreeSet::new(),
            category_ids: BTreeSet::new(),
            payee_ids: BTreeSet::new(),
            accounts: AccountScope::Both,
        };
        let store = QueryStore::new(&connection);
        for kind in [
            ReportKind::Spending,
            ReportKind::IncomeExpense,
            ReportKind::NetWorth,
            ReportKind::BudgetProgress,
        ] {
            let result = store
                .report(
                    budget,
                    &ReportRequest {
                        kind,
                        filter: filter.clone(),
                    },
                )
                .unwrap();
            let metadata = match result {
                ReportResult::Spending(v) => v.presentation,
                ReportResult::IncomeExpense(v) => v.presentation,
                ReportResult::NetWorth(v) => v.presentation,
                ReportResult::BudgetProgress(v) => v.presentation,
            };
            assert!(metadata.is_empty);
            assert_eq!(metadata.row_count, 0);
        }
    }
}
