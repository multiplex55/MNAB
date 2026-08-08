//! Read-only, bounded projection queries.
//!
//! Register paging is keyset based.  The key is deliberately the persisted
//! `(transaction_date, id)` pair: dates are not unique and an offset alone can
//! skip or repeat rows when a transaction is inserted while the user scrolls.
use crate::app::view_model::{BudgetMonthView, CategoryRowView, ViewVersion};
pub use crate::app::view_model::{
    RegisterCursor, RegisterFilter, RegisterRequest as RegisterViewRequest, RegisterScope,
    RegisterSortDirection, RegisterSortField,
};
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
    pub created_at: String,
    pub payee_id: Option<String>,
    pub payee: String,
    pub category_id: Option<String>,
    pub category: String,
    pub memo: Option<String>,
    pub amount: Money,
    pub running_balance: Option<Money>,
    pub cleared_state: String,
    pub approval_state: String,
    pub transfer_id: Option<String>,
    pub split_count: u32,
    pub import_batch_id: Option<String>,
    pub import_source: Option<String>,
    pub review_state: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchHighlightSpan {
    pub field: String,
    pub start: usize,
    pub end: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchRow {
    pub transaction_id: String,
    pub account_id: String,
    pub account: String,
    pub date: String,
    pub payee: String,
    pub category: String,
    pub memo: String,
    pub amount: Money,
    pub approved: bool,
    pub clearance: String,
    pub highlights: Vec<SearchHighlightSpan>,
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
    pub total_matches: u64,
    pub has_more: bool,
    /// Balance immediately before the oldest row in this page, per account.
    pub running_balance_anchors: Vec<(String, Money)>,
    pub reconciliation_separators: Vec<ReconciliationSeparator>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RegisterTotals {
    pub working: Money,
    pub cleared: Money,
    pub reconciled: Money,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountTreeAccount {
    pub id: String,
    pub name: String,
    pub account_type: String,
    pub balance: Money,
    pub closed: bool,
    pub favorite: bool,
    pub last_reconciliation_date: Option<String>,
    pub sort_order: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountTreeGroup {
    pub id: Option<String>,
    pub name: String,
    pub classification: Option<String>,
    pub sort_order: i64,
    pub accounts: Vec<AccountTreeAccount>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountHeader {
    pub account_id: String,
    pub name: String,
    pub account_type: String,
    pub group_path: String,
    pub working: Money,
    pub cleared: Money,
    pub uncleared: Money,
    pub transaction_count: u64,
    pub last_reconciliation_date: Option<String>,
    pub reconciliation_difference: Money,
    pub goal_ids: Vec<String>,
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

    /// Loads the complete tree with balances and reconciliation metadata in one bounded query.
    pub fn account_tree(&self, budget: BudgetId) -> Result<Vec<AccountTreeGroup>, RepositoryError> {
        let mut statement = self.connection.prepare(r#"SELECT g.id,COALESCE(g.name,'Ungrouped'),g.classification,COALESCE(g.sort_order,9223372036854775807),a.id,a.name,a.account_type,a.closed,a.favorite,a.sort_order,COALESCE(SUM(t.amount),0),(SELECT MAX(r.statement_date) FROM reconciliations r WHERE r.account_id=a.id AND r.state='completed')
            FROM accounts a LEFT JOIN account_groups g ON g.id=a.group_id AND g.budget_id=a.budget_id
            LEFT JOIN transactions t ON t.account_id=a.id AND t.archived=0 AND t.voided=0
            WHERE a.budget_id=?1 GROUP BY a.id
            ORDER BY COALESCE(g.sort_order,9223372036854775807),g.id,a.sort_order,a.id"#).map_err(repo)?;
        let rows = statement
            .query_map([budget.to_string()], |r| {
                Ok((
                    r.get::<_, Option<String>>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, i64>(3)?,
                    AccountTreeAccount {
                        id: r.get(4)?,
                        name: r.get(5)?,
                        account_type: r.get(6)?,
                        closed: r.get::<_, i64>(7)? != 0,
                        favorite: r.get::<_, i64>(8)? != 0,
                        sort_order: r.get(9)?,
                        balance: Money::from_minor_units(r.get(10)?),
                        last_reconciliation_date: r.get(11)?,
                    },
                ))
            })
            .map_err(repo)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(repo)?;
        let mut groups: Vec<AccountTreeGroup> = Vec::new();
        for (id, name, classification, sort_order, account) in rows {
            if let Some(group) = groups.iter_mut().find(|g| g.id == id) {
                group.accounts.push(account);
            } else {
                groups.push(AccountTreeGroup {
                    id,
                    name,
                    classification,
                    sort_order,
                    accounts: vec![account],
                });
            }
        }
        Ok(groups)
    }

    pub fn account_header(&self, account: AccountId) -> Result<AccountHeader, RepositoryError> {
        let mut header = self.connection.query_row(r#"SELECT a.id,a.name,a.account_type,COALESCE(g.name,'Ungrouped'),COALESCE(SUM(CASE WHEN t.archived=0 AND t.voided=0 THEN t.amount ELSE 0 END),0),COALESCE(SUM(CASE WHEN t.archived=0 AND t.voided=0 AND t.cleared_state IN ('cleared','reconciled') THEN t.amount ELSE 0 END),0),COALESCE(SUM(CASE WHEN t.archived=0 AND t.voided=0 AND t.cleared_state='uncleared' THEN t.amount ELSE 0 END),0),COUNT(CASE WHEN t.archived=0 THEN 1 END),(SELECT MAX(rr.statement_date) FROM reconciliations rr WHERE rr.account_id=a.id AND rr.state='completed'),COALESCE((SELECT difference FROM reconciliations lr WHERE lr.account_id=a.id ORDER BY statement_date DESC,id DESC LIMIT 1),0)
            FROM accounts a LEFT JOIN account_groups g ON g.id=a.group_id LEFT JOIN transactions t ON t.account_id=a.id WHERE a.id=?1 GROUP BY a.id"#,[account.to_string()],|r| Ok(AccountHeader{account_id:r.get(0)?,name:r.get(1)?,account_type:r.get(2)?,group_path:r.get(3)?,working:Money::from_minor_units(r.get(4)?),cleared:Money::from_minor_units(r.get(5)?),uncleared:Money::from_minor_units(r.get(6)?),transaction_count:r.get::<_,i64>(7)? as u64,last_reconciliation_date:r.get(8)?,reconciliation_difference:Money::from_minor_units(r.get(9)?),goal_ids:vec![]})).map_err(repo)?;
        let mut goals = self
            .connection
            .prepare("SELECT id FROM category_goals WHERE account_id=?1 ORDER BY id")
            .map_err(repo)?;
        header.goal_ids = goals
            .query_map([account.to_string()], |r| r.get(0))
            .map_err(repo)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(repo)?;
        Ok(header)
    }

    pub fn register_projection(
        &self,
        account: AccountId,
        offset: u32,
        requested_size: u32,
        generation: crate::storage::worker::Generation,
    ) -> Result<crate::app::view_model::RegisterPageView, RepositoryError> {
        let budget_id = self
            .connection
            .query_row(
                "SELECT budget_id FROM accounts WHERE id=?1",
                [account.to_string()],
                |r| r.get::<_, String>(0),
            )
            .map_err(repo)?
            .parse()
            .map_err(repo)?;
        let request = RegisterViewRequest {
            budget_id,
            scope: RegisterScope::Account(account),
            filter: RegisterFilter::default(),
            sort_field: RegisterSortField::Date,
            sort_direction: RegisterSortDirection::Descending,
            page_size: requested_size as usize,
            cursor: None,
        };
        let page = self.register_page(&request)?;
        let _ = offset;
        crate::storage::mapping::register_page(page, request, generation)
    }

    /// Executes a report inside SQLite. Only typed aggregate rows cross this boundary; ledger
    /// rows remain owned by the storage thread.
    pub fn report(
        &self,
        budget: BudgetId,
        request: &ReportRequest,
    ) -> Result<ReportResult, RepositoryError> {
        match request.kind {
            ReportKind::Spending
            | ReportKind::SpendingByCategory
            | ReportKind::SpendingByPayee
            | ReportKind::MonthlySpendingTrend => self
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
            ReportKind::Spending
            | ReportKind::SpendingByCategory
            | ReportKind::SpendingByPayee
            | ReportKind::MonthlySpendingTrend
            | ReportKind::IncomeExpense => {
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

    pub fn register_view(
        &self,
        request: &RegisterViewRequest,
    ) -> Result<RegisterPage, RepositoryError> {
        self.register_page(request)
    }

    pub fn search(&self, expression: &str, limit: u32) -> Result<Vec<SearchRow>, RepositoryError> {
        let ast = crate::app::search::parse(expression).map_err(|diagnostics| {
            repo(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                diagnostics
                    .first()
                    .map_or("invalid search expression".into(), |d| d.message.clone()),
            ))
        })?;
        let (filter_sql, mut values) = transaction_filter_sql(&ast, "t");
        values.push(i64::from(limit.clamp(1, 100)).into());
        let sql = format!(
            r#"SELECT t.id,t.account_id,a.name,t.transaction_date,
                      COALESCE(p.name,t.payee_snapshot,''),COALESCE(c.name,''),COALESCE(t.memo,''),t.amount,
                      t.approval_state,t.cleared_state
               FROM transactions t
               JOIN accounts a ON a.id=t.account_id
               LEFT JOIN account_groups g ON g.id=a.group_id
               LEFT JOIN payees p ON p.id=t.payee_id
               LEFT JOIN categories c ON c.id=t.category_id
               WHERE t.archived=0 AND COALESCE(t.voided,0)=0 {filter_sql}
               ORDER BY t.transaction_date DESC,t.id DESC LIMIT ?"#
        );
        let mut stmt = self.connection.prepare(&sql).map_err(repo)?;
        let terms = ast.terms.clone();
        stmt.query_map(params_from_iter(values), |r| {
            let account: String = r.get(2)?;
            let payee: String = r.get(4)?;
            let category: String = r.get(5)?;
            let memo: String = r.get(6)?;
            Ok(SearchRow {
                transaction_id: r.get(0)?,
                account_id: r.get(1)?,
                account: account.clone(),
                date: r.get(3)?,
                payee: payee.clone(),
                category: category.clone(),
                memo: memo.clone(),
                amount: Money::from_minor_units(r.get(7)?),
                approved: r.get::<_, String>(8)? == "approved",
                clearance: r.get(9)?,
                highlights: highlight_spans(&terms, &account, &payee, &category, &memo),
            })
        })
        .map_err(repo)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(repo)
    }

    /// Compatibility helper. Unlike the old unbounded query this returns at most one page.
    pub fn register(&self, account: AccountId) -> Result<Vec<RegisterRow>, RepositoryError> {
        let budget = self
            .connection
            .query_row(
                "SELECT budget_id FROM accounts WHERE id=?1",
                [account.to_string()],
                |r| r.get::<_, String>(0),
            )
            .map_err(repo)?
            .parse()
            .map_err(repo)?;
        Ok(self
            .register_page(&RegisterViewRequest {
                budget_id: budget,
                scope: RegisterScope::Account(account),
                filter: RegisterFilter::default(),
                sort_field: RegisterSortField::Date,
                sort_direction: RegisterSortDirection::Descending,
                page_size: MAX_REGISTER_PAGE_SIZE,
                cursor: None,
            })?
            .rows)
    }

    /// Executes the canonical, fully-identifying register request.
    pub fn register_page(
        &self,
        request: &RegisterViewRequest,
    ) -> Result<RegisterPage, RepositoryError> {
        let limit = request.page_size.clamp(1, MAX_REGISTER_PAGE_SIZE);
        let mut clauses = vec![
            "t.archived=0".to_owned(),
            "t.voided=0".to_owned(),
            "t.budget_id=?".to_owned(),
        ];
        let mut values = vec![Value::Text(request.budget_id.to_string())];
        if let RegisterScope::Account(id) = request.scope {
            clauses.push("t.account_id=?".into());
            values.push(id.to_string().into());
        }
        if let Some(v) = request.filter.from {
            clauses.push("t.transaction_date>=?".into());
            values.push(v.to_string().into());
        }
        if let Some(v) = request.filter.through {
            clauses.push("t.transaction_date<=?".into());
            values.push(v.to_string().into());
        }
        if let Some(v) = &request.filter.cleared_state {
            clauses.push("t.cleared_state=?".into());
            values.push(v.clone().into());
        }
        if let Some(v) = &request.filter.approval_state {
            clauses.push("t.approval_state=?".into());
            values.push(v.clone().into());
        }
        if let Some(v) = request.filter.minimum_amount_cents {
            clauses.push("t.amount>=?".into());
            values.push(v.into());
        }
        if let Some(v) = request.filter.maximum_amount_cents {
            clauses.push("t.amount<=?".into());
            values.push(v.into());
        }
        let categories = request
            .filter
            .category_ids
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        let payees = request
            .filter
            .payee_ids
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        if !categories.is_empty() {
            clauses.push(format!(
                "t.category_id IN ({})",
                vec!["?"; categories.len()].join(",")
            ));
            values.extend(categories.into_iter().map(Value::Text));
        }
        if !payees.is_empty() {
            clauses.push(format!(
                "t.payee_id IN ({})",
                vec!["?"; payees.len()].join(",")
            ));
            values.extend(payees.into_iter().map(Value::Text));
        }
        if !request.filter.search.trim().is_empty() {
            let ast = crate::app::search::parse(&request.filter.search).map_err(|d| {
                repo(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    d.first()
                        .map_or_else(|| "invalid register filter".into(), |x| x.message.clone()),
                ))
            })?;
            let (sql, binds) = transaction_filter_sql(&ast, "t");
            if !sql.is_empty() {
                clauses.push(sql.trim_start_matches(" AND ").into());
                values.extend(binds);
            }
        }
        let descending = request.sort_direction == RegisterSortDirection::Descending;
        let total_where = clauses.join(" AND ");
        let total_matches = self.connection.query_row(&format!("SELECT COUNT(*) FROM transactions t JOIN accounts a ON a.id=t.account_id LEFT JOIN account_groups g ON g.id=a.group_id LEFT JOIN payees p ON p.id=t.payee_id LEFT JOIN categories c ON c.id=t.category_id WHERE {total_where}"), params_from_iter(values.clone()), |r| r.get::<_, i64>(0)).map_err(repo)? as u64;
        if let Some(cursor) = &request.cursor {
            let op = if descending { "<" } else { ">" };
            clauses.push(format!("(t.transaction_date {op} ? OR (t.transaction_date=? AND (t.created_at {op} ? OR (t.created_at=? AND t.id {op} ?))))"));
            values.extend([
                cursor.date.to_string().into(),
                cursor.date.to_string().into(),
                cursor.created_at.clone().into(),
                cursor.created_at.clone().into(),
                cursor.transaction_id.to_string().into(),
            ]);
        }
        let base_where = clauses.join(" AND ");
        values.push((limit as i64 + 1).into());
        let direction = if descending { "DESC" } else { "ASC" };
        let balance = if matches!(request.scope, RegisterScope::Account(_)) {
            "(SELECT COALESCE(SUM(rb.amount),0) FROM transactions rb WHERE rb.account_id=t.account_id AND rb.archived=0 AND rb.voided=0 AND (rb.transaction_date<t.transaction_date OR (rb.transaction_date=t.transaction_date AND (rb.created_at<t.created_at OR (rb.created_at=t.created_at AND rb.id<=t.id)))))"
        } else {
            "NULL"
        };
        let sql = format!(
            r#"SELECT t.id,t.account_id,a.name,t.transaction_date,t.created_at,t.payee_id,COALESCE(p.name,t.payee_snapshot,''),t.category_id,COALESCE(c.name,''),t.memo,t.amount,{balance},t.cleared_state,t.approval_state,t.transfer_id,(SELECT COUNT(*) FROM subtransactions s WHERE s.transaction_id=t.id),t.import_batch_id,ib.source_name,ib.state
          FROM transactions t JOIN accounts a ON a.id=t.account_id LEFT JOIN account_groups g ON g.id=a.group_id LEFT JOIN payees p ON p.id=t.payee_id LEFT JOIN categories c ON c.id=t.category_id LEFT JOIN import_batches ib ON ib.id=t.import_batch_id
          WHERE {base_where} ORDER BY t.transaction_date {direction},t.created_at {direction},t.id {direction} LIMIT ?"#
        );
        let mut stmt = self.connection.prepare(&sql).map_err(repo)?;
        let mut rows = stmt
            .query_map(params_from_iter(values), |r| {
                Ok(RegisterRow {
                    transaction_id: r.get(0)?,
                    account_id: r.get(1)?,
                    account_name: r.get(2)?,
                    date: r.get(3)?,
                    created_at: r.get(4)?,
                    payee_id: r.get(5)?,
                    payee: r.get(6)?,
                    category_id: r.get(7)?,
                    category: r.get(8)?,
                    memo: r.get(9)?,
                    amount: Money::from_minor_units(r.get(10)?),
                    running_balance: r.get::<_, Option<i64>>(11)?.map(Money::from_minor_units),
                    cleared_state: r.get(12)?,
                    approval_state: r.get(13)?,
                    transfer_id: r.get(14)?,
                    split_count: r.get::<_, i64>(15)? as u32,
                    import_batch_id: r.get(16)?,
                    import_source: r.get(17)?,
                    review_state: r.get(18)?,
                })
            })
            .map_err(repo)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(repo)?;
        let has_more = rows.len() > limit;
        rows.truncate(limit);
        let next_cursor = has_more
            .then(|| rows.last())
            .flatten()
            .map(|r| RegisterCursor {
                date: time::Date::parse(
                    &r.date,
                    &time::format_description::well_known::Iso8601::DATE,
                )
                .expect("validated database date"),
                created_at: r.created_at.clone(),
                transaction_id: r.transaction_id.parse().expect("validated database id"),
            });
        Ok(RegisterPage {
            rows,
            next_cursor,
            total_matches,
            has_more,
            running_balance_anchors: vec![],
            reconciliation_separators: vec![],
        })
    }

    pub fn budget_month(
        &self,
        budget: BudgetId,
        month: &str,
    ) -> Result<BudgetMonthView, RepositoryError> {
        let parsed_month = parse_month(month)?;
        let mut stmt = self.connection.prepare(
            r"SELECT g.id,c.id,g.name,c.name,g.sort_order,c.sort_order,
                      COALESCE(g.hidden,0),COALESCE(c.hidden,0),COALESCE(c.archived,0),
                      COALESCE(a.amount,0),
                      COALESCE((SELECT SUM(CASE WHEN x.amount<0 THEN x.amount ELSE 0 END) FROM (
                          SELECT t.category_id,t.amount FROM transactions t WHERE t.budget_id=?1 AND substr(t.transaction_date,1,7)=?2 AND t.archived=0 AND COALESCE(t.voided,0)=0
                          UNION ALL SELECT s.category_id,s.amount FROM subtransactions s JOIN transactions t ON t.id=s.transaction_id WHERE t.budget_id=?1 AND substr(t.transaction_date,1,7)=?2 AND t.archived=0 AND COALESCE(t.voided,0)=0
                      ) x WHERE x.category_id=c.id),0) activity,
                      t.id,t.amount,t.due_date,t.target_type,
                      CASE WHEN m.category_id IS NULL THEN 0 ELSE 1 END protected
               FROM category_groups g JOIN categories c ON c.group_id=g.id
               LEFT JOIN budget_assignments a ON a.category_id=c.id AND a.budget_month=?2
               LEFT JOIN targets t ON t.category_id=c.id
               LEFT JOIN credit_card_payment_categories m ON m.category_id=c.id
               WHERE g.budget_id=?1
               ORDER BY g.sort_order,g.id,c.sort_order,c.id",
        ).map_err(repo)?;
        let mut assigned = 0_i64;
        let mut activity_total = 0_i64;
        let rows = stmt.query_map((budget.to_string(), month), |r| {
            let assigned_cents: i64 = r.get(9)?;
            let activity_cents: i64 = r.get(10)?;
            let target_amount: Option<i64> = r.get(12)?;
            let available = assigned_cents + activity_cents;
            let underfunded = target_amount.map_or(0, |t| (t - assigned_cents).max(0));
            Ok(CategoryRowView {
                group_id: r.get::<_, String>(0)?.parse().map_err(|_| rusqlite::Error::InvalidQuery)?, category_id: r.get::<_, String>(1)?.parse().map_err(|_| rusqlite::Error::InvalidQuery)?,
                group_name: r.get(2)?, name: r.get(3)?, group_sort: r.get(4)?, category_sort: r.get(5)?,
                group_collapsed: r.get::<_, i64>(6)? != 0, hidden: r.get::<_, i64>(7)? != 0, archived: r.get::<_, i64>(8)? != 0,
                assigned_cents, activity_cents, available_cents: available, overspending_cents: (-available).max(0), underfunded_cents: underfunded,
                target_id: r.get::<_, Option<String>>(11)?.map(|x| x.parse().map_err(|_| rusqlite::Error::InvalidQuery)).transpose()?, target_amount_cents: target_amount,
                target_remaining_cents: target_amount.map(|_| underfunded), target_due_date: r.get(13)?, target_status: if underfunded == 0 { "funded" } else { "underfunded" }.into(),
                credit_card_payment: r.get::<_, i64>(15)? != 0, protected: r.get::<_, i64>(15)? != 0,
                inspector: format!("Assigned {assigned_cents}¢, activity {activity_cents}¢, available {available}¢. Recommendations are advisory and require Apply."),
            })
        }).map_err(repo)?.collect::<Result<Vec<_>, _>>().map_err(repo)?;
        for row in &rows {
            assigned += row.assigned_cents;
            activity_total += row.activity_cents;
        }
        let available = assigned + activity_total;
        let revision = self.connection.query_row(
            "SELECT COALESCE(MAX(rev),0) FROM (SELECT MAX(strftime('%s',modified_at)) rev FROM budget_assignments WHERE budget_id=?1 UNION ALL SELECT MAX(strftime('%s',modified_at)) FROM transactions WHERE budget_id=?1 UNION ALL SELECT MAX(sort_order) FROM categories WHERE budget_id=?1)",
            [budget.to_string()], |r| r.get::<_, Option<u64>>(0)).map_err(repo)?.unwrap_or(0);
        Ok(BudgetMonthView { version: ViewVersion { generation: 0, revision }, month: parsed_month, calculation_revision: revision, ready_to_assign_cents: -assigned, assigned_cents: assigned, activity_cents: activity_total, available_cents: available, overspending_cents: rows.iter().map(|r| r.overspending_cents).sum(), rows, inspector: vec!["Ready to Assign is calculated from persisted cents; targets never move money without a confirmed assignment command.".into()] })
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
            clauses.push("a.account_type IN ('loan','asset','liability')".into())
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
        " AND a.account_type IN ('loan','asset','liability')"
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

fn escape_like(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}
fn transaction_filter_sql(
    ast: &crate::app::search::SearchAst,
    alias: &str,
) -> (String, Vec<Value>) {
    use crate::app::search::{Comparison, SearchTerm};
    let mut clauses = Vec::new();
    let mut values = Vec::new();
    for term in &ast.terms {
        match term {
            SearchTerm::Text(v) => {
                clauses.push(format!("(LOWER(COALESCE(p.name,{alias}.payee_snapshot,'')||' '||COALESCE(c.name,'')||' '||COALESCE({alias}.memo,'')||' '||a.name) LIKE ? ESCAPE '\\\\')"));
                values.push(format!("%{}%", escape_like(&v.to_lowercase())).into());
            }
            SearchTerm::Account(v)
            | SearchTerm::AccountGroup(v)
            | SearchTerm::Category(v)
            | SearchTerm::Payee(v)
            | SearchTerm::Memo(v) => {
                let col = match term {
                    SearchTerm::Account(_) => "a.name",
                    SearchTerm::AccountGroup(_) => "COALESCE(g.name,'')",
                    SearchTerm::Category(_) => "COALESCE(c.name,'')",
                    SearchTerm::Payee(_) => &format!("COALESCE(p.name,{alias}.payee_snapshot,'')"),
                    _ => &format!("COALESCE({alias}.memo,'')"),
                };
                clauses.push(format!("LOWER({col}) LIKE ? ESCAPE '\\\\'"));
                values.push(format!("%{}%", escape_like(&v.to_lowercase())).into());
            }
            SearchTerm::Amount { comparison, value } => {
                let op = match comparison {
                    Comparison::Less => "<",
                    Comparison::LessEqual => "<=",
                    Comparison::Equal => "=",
                    Comparison::GreaterEqual => ">=",
                    Comparison::Greater => ">",
                };
                clauses.push(format!("{alias}.amount {op} ?"));
                values.push(value.minor_units().into());
            }
            SearchTerm::Before(v) => {
                clauses.push(format!("{alias}.transaction_date < ?"));
                values.push(v.to_string().into());
            }
            SearchTerm::After(v) => {
                clauses.push(format!("{alias}.transaction_date > ?"));
                values.push(v.to_string().into());
            }
            SearchTerm::From(v) | SearchTerm::Through(v) => {
                clauses.push(format!(
                    "{alias}.transaction_date {} ?",
                    if matches!(term, SearchTerm::From(_)) {
                        ">="
                    } else {
                        "<="
                    }
                ));
                values.push(v.to_string().into());
            }
            SearchTerm::Uncategorized(v) => clauses.push(format!(
                "{alias}.category_id IS {}NULL",
                if *v { "" } else { "NOT " }
            )),
            SearchTerm::Reconciled(v) => {
                clauses.push(format!(
                    "{alias}.cleared_state {} ?",
                    if *v { "=" } else { "<>" }
                ));
                values.push(String::from("reconciled").into());
            }
            SearchTerm::Imported(v) => clauses.push(format!(
                "{alias}.import_batch_id IS {}NULL",
                if *v { "NOT " } else { "" }
            )),
            SearchTerm::Transfer(v) => clauses.push(format!(
                "{alias}.transfer_id IS {}NULL",
                if *v { "NOT " } else { "" }
            )),
            SearchTerm::Cleared(v) => {
                clauses.push(if *v {
                    format!("{alias}.cleared_state <> ?")
                } else {
                    format!("{alias}.cleared_state = ?")
                });
                values.push(String::from("uncleared").into());
            }
            SearchTerm::Approved(v) => {
                clauses.push(format!("{alias}.approval_state = ?"));
                values.push(String::from(if *v { "approved" } else { "unapproved" }).into());
            }
        }
    }
    (
        if clauses.is_empty() {
            String::new()
        } else {
            format!(" AND {}", clauses.join(" AND "))
        },
        values,
    )
}

fn highlight_spans(
    terms: &[crate::app::search::SearchTerm],
    account: &str,
    payee: &str,
    category: &str,
    memo: &str,
) -> Vec<SearchHighlightSpan> {
    let mut out = Vec::new();
    for term in terms {
        let (field, text) = match term {
            crate::app::search::SearchTerm::Text(v) => ("payee", v.as_str()),
            crate::app::search::SearchTerm::Account(v) => ("account", v.as_str()),
            crate::app::search::SearchTerm::AccountGroup(v) => ("account", v.as_str()),
            crate::app::search::SearchTerm::Category(v) => ("category", v.as_str()),
            crate::app::search::SearchTerm::Payee(v) => ("payee", v.as_str()),
            crate::app::search::SearchTerm::Memo(v) => ("memo", v.as_str()),
            _ => continue,
        };
        let hay = match field {
            "account" => account,
            "category" => category,
            "memo" => memo,
            _ => payee,
        };
        if let Some(byte) = hay.to_lowercase().find(&text.to_lowercase()) {
            let end = byte
                + hay[byte..]
                    .chars()
                    .take(text.chars().count())
                    .map(char::len_utf8)
                    .sum::<usize>();
            if hay.is_char_boundary(byte) && hay.is_char_boundary(end) {
                out.push(SearchHighlightSpan {
                    field: field.into(),
                    start: byte,
                    end,
                });
            }
        }
    }
    out
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
            ReportKind::SpendingByCategory,
            ReportKind::SpendingByPayee,
            ReportKind::MonthlySpendingTrend,
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

#[cfg(test)]
mod large_fixture_tests {
    use super::*;

    fn fixture(row_count: usize) -> (tempfile::TempDir, Connection, BudgetId, AccountId) {
        let directory = tempfile::tempdir().unwrap();
        let mut connection =
            crate::storage::connection::open_primary(&directory.path().join("large.sqlite3"))
                .unwrap();
        let budget = BudgetId::new();
        let account = AccountId::new();
        connection.execute("INSERT INTO budgets(id,name,created_at,modified_at,archived) VALUES(?1,'Large',datetime('now'),datetime('now'),0)",[budget.to_string()]).unwrap();
        connection.execute("INSERT INTO accounts(id,budget_id,name,account_type,sort_order,created_at,modified_at) VALUES(?1,?2,'Checking','checking',0,datetime('now'),datetime('now'))",(account.to_string(),budget.to_string())).unwrap();
        let transaction = connection.transaction().unwrap();
        {
            let mut insert = transaction.prepare("INSERT INTO transactions(id,budget_id,account_id,transaction_date,amount,cleared_state,approval_state,created_at,modified_at) VALUES(?1,?2,?3,?4,1,'uncleared','approved',datetime('now'),datetime('now'))").unwrap();
            for index in 0..row_count {
                // Repeated dates exercise the id tie-breaker as well as date progression.
                let day = index % 28 + 1;
                let month = index / 28 % 12 + 1;
                insert
                    .execute((
                        crate::domain::TransactionId::new().to_string(),
                        budget.to_string(),
                        account.to_string(),
                        format!("2026-{month:02}-{day:02}"),
                    ))
                    .unwrap();
            }
        }
        transaction.commit().unwrap();
        (directory, connection, budget, account)
    }

    #[test]
    fn large_register_pages_are_bounded_and_cursor_progression_is_stable() {
        let (_directory, connection, budget, account) = fixture(5_000);
        let store = QueryStore::new(&connection);
        let base = RegisterViewRequest {
            budget_id: budget,
            scope: RegisterScope::AllTransactions,
            filter: RegisterFilter::default(),
            sort_field: RegisterSortField::Date,
            sort_direction: RegisterSortDirection::Descending,
            page_size: usize::MAX,
            cursor: None,
        };
        let first = store.register_page(&base).unwrap();
        assert_eq!(first.rows.len(), MAX_REGISTER_PAGE_SIZE);
        let cursor = first.next_cursor.as_ref().unwrap();
        let second_request = RegisterViewRequest {
            scope: RegisterScope::Account(account),
            cursor: Some(cursor.clone()),
            page_size: MAX_REGISTER_PAGE_SIZE,
            ..base
        };
        let second = store.register_page(&second_request).unwrap();
        assert_eq!(second.rows.len(), MAX_REGISTER_PAGE_SIZE);
        let cursor_key = (
            cursor.date.to_string(),
            cursor.created_at.clone(),
            cursor.transaction_id.to_string(),
        );
        assert!(second.rows.iter().all(|row| (
            row.date.clone(),
            row.created_at.clone(),
            row.transaction_id.clone()
        ) < cursor_key));
        assert!(first.rows.iter().all(|row| {
            !second
                .rows
                .iter()
                .any(|next| next.transaction_id == row.transaction_id)
        }));
    }

    #[test]
    fn large_register_plan_uses_the_forward_migration_index() {
        let (_directory, connection, budget, _account) = fixture(1_000);
        let mut statement = connection.prepare(
            "EXPLAIN QUERY PLAN SELECT id FROM transactions WHERE budget_id=?1 AND archived=0 AND (transaction_date<?2 OR (transaction_date=?2 AND id<?3)) ORDER BY transaction_date DESC,id DESC LIMIT 201",
        ).unwrap();
        let plan = statement
            .query_map(
                (
                    budget.to_string(),
                    "2026-12-31",
                    "ffffffff-ffff-ffff-ffff-ffffffffffff",
                ),
                |row| row.get::<_, String>(3),
            )
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
            .join("\n");
        // Assert only the required access path, not SQLite's complete/version-specific plan.
        assert!(
            plan.contains("idx_transactions_budget_register_page"),
            "{plan}"
        );
        assert!(!plan.contains("SCAN transactions"), "{plan}");
    }
}
