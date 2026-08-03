//! Immutable report requests, results, and pure ledger aggregations.
use super::*;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use time::{Date, OffsetDateTime};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DateRange {
    pub start: Date,
    pub end: Date,
}
impl DateRange {
    #[must_use]
    pub fn contains(self, d: Date) -> bool {
        d >= self.start && d <= self.end
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub enum AccountScope {
    OnBudget,
    Tracking,
    #[default]
    Both,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReportFilter {
    pub dates: DateRange,
    pub account_ids: BTreeSet<AccountId>,
    pub category_group_ids: BTreeSet<CategoryGroupId>,
    pub category_ids: BTreeSet<CategoryId>,
    pub payee_ids: BTreeSet<PayeeId>,
    pub accounts: AccountScope,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ReportKind {
    Spending,
    IncomeExpense,
    NetWorth,
    BudgetProgress,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReportRequest {
    pub kind: ReportKind,
    pub filter: ReportFilter,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceData {
    pub revision: u64,
    pub refreshed_at: OffsetDateTime,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReportData<'a> {
    pub source: SourceData,
    pub accounts: &'a [Account],
    pub groups: &'a [CategoryGroup],
    pub categories: &'a [Category],
    pub payees: &'a [Payee],
    pub transactions: &'a [Transaction],
    pub assignments: &'a [BudgetAssignment],
    pub targets: &'a [Target],
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct OwnedReportData {
    pub source: SourceData,
    pub accounts: Vec<Account>,
    pub groups: Vec<CategoryGroup>,
    pub categories: Vec<Category>,
    pub payees: Vec<Payee>,
    pub transactions: Vec<Transaction>,
    pub assignments: Vec<BudgetAssignment>,
    pub targets: Vec<Target>,
}
impl OwnedReportData {
    #[must_use]
    pub fn as_data(&self) -> ReportData<'_> {
        ReportData {
            source: self.source,
            accounts: &self.accounts,
            groups: &self.groups,
            categories: &self.categories,
            payees: &self.payees,
            transactions: &self.transactions,
            assignments: &self.assignments,
            targets: &self.targets,
        }
    }
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SpendingRow {
    pub group_id: CategoryGroupId,
    pub category_id: CategoryId,
    pub amount: Money,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PayeeSpendingRow {
    pub payee_id: Option<PayeeId>,
    pub amount: Money,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MonthlySpendingRow {
    pub month: BudgetMonth,
    pub amount: Money,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SpendingResult {
    pub source: SourceData,
    pub rows: Vec<SpendingRow>,
    pub groups: Vec<(CategoryGroupId, Money)>,
    pub monthly: Vec<MonthlySpendingRow>,
    pub payees: Vec<PayeeSpendingRow>,
    pub total: Money,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct IncomeExpenseRow {
    pub month: BudgetMonth,
    pub income: Money,
    pub expense: Money,
    pub net: Money,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct IncomeExpenseResult {
    pub source: SourceData,
    pub rows: Vec<IncomeExpenseRow>,
    pub income: Money,
    pub expense: Money,
    pub net: Money,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NetWorthRow {
    pub date: Date,
    pub assets: Money,
    pub liabilities: Money,
    pub net_worth: Money,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NetWorthResult {
    pub source: SourceData,
    pub included_accounts: Vec<AccountId>,
    pub rows: Vec<NetWorthRow>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BudgetProgressRow {
    pub month: BudgetMonth,
    pub category_id: CategoryId,
    pub assigned: Money,
    pub spent: Money,
    pub target: Option<Money>,
    pub target_completion_basis_points: Option<u16>,
    pub underfunded: Money,
    pub overspent: Money,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BudgetProgressResult {
    pub source: SourceData,
    pub rows: Vec<BudgetProgressRow>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ReportResult {
    Spending(SpendingResult),
    IncomeExpense(IncomeExpenseResult),
    NetWorth(NetWorthResult),
    BudgetProgress(BudgetProgressResult),
}

/// UI-owned report lifecycle. `accept` is the single gate through which worker results may
/// replace a displayed immutable snapshot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReportLoadStatus {
    Idle,
    Loading,
    Ready,
    Cancelled,
    Failed(String),
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReportLoadState {
    pub generation: u64,
    pub status: ReportLoadStatus,
    pub request: Option<ReportRequest>,
    pub displayed: Option<ReportResult>,
    pub last_refreshed: Option<OffsetDateTime>,
}
impl Default for ReportLoadState {
    fn default() -> Self {
        Self {
            generation: 0,
            status: ReportLoadStatus::Idle,
            request: None,
            displayed: None,
            last_refreshed: None,
        }
    }
}
impl ReportLoadState {
    pub fn begin(&mut self, request: ReportRequest) -> u64 {
        self.generation = self.generation.wrapping_add(1);
        self.request = Some(request);
        self.status = ReportLoadStatus::Loading;
        self.generation
    }
    pub fn cancel(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.status = ReportLoadStatus::Cancelled
    }
    pub fn fail(&mut self, generation: u64, message: impl Into<String>) -> bool {
        if generation != self.generation {
            return false;
        }
        self.status = ReportLoadStatus::Failed(message.into());
        true
    }
    pub fn retry(&mut self) -> Option<(u64, ReportRequest)> {
        let request = self.request.clone()?;
        let generation = self.begin(request.clone());
        Some((generation, request))
    }
    pub fn accept(&mut self, generation: u64, result: ReportResult) -> bool {
        if generation != self.generation || !matches!(self.status, ReportLoadStatus::Loading) {
            return false;
        }
        self.last_refreshed = Some(match &result {
            ReportResult::Spending(x) => x.source.refreshed_at,
            ReportResult::IncomeExpense(x) => x.source.refreshed_at,
            ReportResult::NetWorth(x) => x.source.refreshed_at,
            ReportResult::BudgetProgress(x) => x.source.refreshed_at,
        });
        self.displayed = Some(result);
        self.status = ReportLoadStatus::Ready;
        true
    }
    pub fn export_displayed(&self) -> Result<String, ExportError> {
        export_csv(
            self.displayed
                .as_ref()
                .ok_or(ExportError::NoDisplayedResult)?,
        )
        .map_err(|e| ExportError::Csv(e.to_string()))
    }
}
#[derive(Clone, Debug, thiserror::Error, Eq, PartialEq)]
pub enum ExportError {
    #[error("there is no displayed report to export")]
    NoDisplayedResult,
    #[error("CSV export failed: {0}")]
    Csv(String),
}

fn month(d: Date) -> BudgetMonth {
    BudgetMonth::new(d.year(), u8::from(d.month())).expect("calendar month")
}
fn add<K: Ord>(map: &mut BTreeMap<K, Money>, key: K, value: Money) {
    let old = map.get(&key).copied().unwrap_or(Money::ZERO);
    map.insert(key, old.checked_add(value).expect("report total overflow"));
}
fn selected_account(a: &Account, f: &ReportFilter) -> bool {
    (f.account_ids.is_empty() || f.account_ids.contains(&a.id))
        && match f.accounts {
            AccountScope::Both => true,
            AccountScope::OnBudget => a.classification() == AccountClassification::OnBudget,
            AccountScope::Tracking => a.classification() == AccountClassification::Tracking,
        }
}
fn lines(t: &Transaction) -> Vec<(Option<CategoryId>, Money)> {
    match &t.body {
        TransactionBody::Categorized { category_id }
        | TransactionBody::OpeningBalance {
            category_id: Some(category_id),
        } => vec![(Some(*category_id), t.amount)],
        TransactionBody::Split { lines } => lines
            .iter()
            .map(|l| (Some(l.category_id), l.amount))
            .collect(),
        TransactionBody::OpeningBalance { category_id: None }
        | TransactionBody::Transfer { .. } => vec![(None, t.amount)],
    }
}
fn filtered<'a>(
    d: &'a ReportData<'a>,
    f: &'a ReportFilter,
    budget_only: bool,
) -> impl Iterator<Item = &'a Transaction> {
    d.transactions.iter().filter(move |t| {
        !t.archived
            && !t.voided
            && f.dates.contains(t.date.0)
            && d.accounts
                .iter()
                .find(|a| a.id == t.account_id)
                .is_some_and(|a| {
                    selected_account(a, f)
                        && (!budget_only || a.classification() == AccountClassification::OnBudget)
                })
            && (f.payee_ids.is_empty() || t.payee_id.is_some_and(|p| f.payee_ids.contains(&p)))
    })
}
fn category_ok(d: &ReportData<'_>, f: &ReportFilter, c: CategoryId) -> bool {
    (f.category_ids.is_empty() || f.category_ids.contains(&c))
        && (f.category_group_ids.is_empty()
            || d.categories
                .iter()
                .find(|x| x.id == c)
                .is_some_and(|x| f.category_group_ids.contains(&x.group_id)))
}

#[must_use]
pub fn spending(data: &ReportData<'_>, filter: &ReportFilter) -> SpendingResult {
    let mut cats = BTreeMap::new();
    let mut months = BTreeMap::new();
    let mut payees = BTreeMap::new();
    for t in filtered(data, filter, true) {
        for (c, a) in lines(t) {
            if a < Money::ZERO && c.is_some_and(|id| category_ok(data, filter, id)) {
                let spend = a.checked_neg().expect("negation");
                let id = c.unwrap();
                add(&mut cats, id, spend);
                add(&mut months, month(t.date.0), spend);
                add(&mut payees, t.payee_id, spend);
            }
        }
    }
    let rows = cats
        .into_iter()
        .filter_map(|(category_id, amount)| {
            data.categories
                .iter()
                .find(|c| c.id == category_id)
                .map(|c| SpendingRow {
                    group_id: c.group_id,
                    category_id,
                    amount,
                })
        })
        .collect::<Vec<_>>();
    let mut groups = BTreeMap::new();
    for r in &rows {
        add(&mut groups, r.group_id, r.amount)
    }
    let total = rows.iter().fold(Money::ZERO, |s, r| {
        s.checked_add(r.amount).expect("overflow")
    });
    SpendingResult {
        source: data.source,
        rows,
        groups: groups.into_iter().collect(),
        monthly: months
            .into_iter()
            .map(|(month, amount)| MonthlySpendingRow { month, amount })
            .collect(),
        payees: payees
            .into_iter()
            .map(|(payee_id, amount)| PayeeSpendingRow { payee_id, amount })
            .collect(),
        total,
    }
}
#[must_use]
pub fn income_expense(data: &ReportData<'_>, filter: &ReportFilter) -> IncomeExpenseResult {
    let mut map: BTreeMap<BudgetMonth, (Money, Money)> = BTreeMap::new();
    for t in filtered(data, filter, true) {
        for (c, a) in lines(t) {
            if c.is_some_and(|x| category_ok(data, filter, x)) {
                let e = map.entry(month(t.date.0)).or_default();
                if a >= Money::ZERO {
                    e.0 = e.0.checked_add(a).unwrap()
                } else {
                    e.1 = e.1.checked_add(a.checked_neg().unwrap()).unwrap()
                }
            }
        }
    }
    let rows = map
        .into_iter()
        .map(|(month, (income, expense))| IncomeExpenseRow {
            month,
            income,
            expense,
            net: income.checked_sub(expense).unwrap(),
        })
        .collect::<Vec<_>>();
    let income = rows
        .iter()
        .fold(Money::ZERO, |s, r| s.checked_add(r.income).unwrap());
    let expense = rows
        .iter()
        .fold(Money::ZERO, |s, r| s.checked_add(r.expense).unwrap());
    IncomeExpenseResult {
        source: data.source,
        rows,
        income,
        expense,
        net: income.checked_sub(expense).unwrap(),
    }
}
#[must_use]
pub fn net_worth(data: &ReportData<'_>, filter: &ReportFilter) -> NetWorthResult {
    let included = data
        .accounts
        .iter()
        .filter(|a| selected_account(a, filter))
        .map(|a| a.id)
        .collect::<BTreeSet<_>>();
    let mut by_date = BTreeMap::new();
    for t in data.transactions.iter().filter(|t| {
        !t.archived && !t.voided && t.date.0 <= filter.dates.end && included.contains(&t.account_id)
    }) {
        add(&mut by_date, t.date.0, t.amount)
    }
    let mut balance = Money::ZERO;
    let mut rows = vec![];
    for (date, delta) in by_date {
        balance = balance.checked_add(delta).unwrap();
        if date >= filter.dates.start {
            let mut assets = Money::ZERO;
            let mut liabilities = Money::ZERO;
            let mut balances = BTreeMap::new();
            for t in data.transactions.iter().filter(|t| {
                !t.archived && !t.voided && t.date.0 <= date && included.contains(&t.account_id)
            }) {
                add(&mut balances, t.account_id, t.amount)
            }
            for (a, v) in balances {
                let ty = data
                    .accounts
                    .iter()
                    .find(|x| x.id == a)
                    .unwrap()
                    .account_type;
                if matches!(
                    ty,
                    AccountType::CreditCard | AccountType::Loan | AccountType::Liability
                ) {
                    liabilities = liabilities.checked_add(v).unwrap()
                } else {
                    assets = assets.checked_add(v).unwrap()
                }
            }
            rows.push(NetWorthRow {
                date,
                assets,
                liabilities,
                net_worth: assets.checked_add(liabilities).unwrap(),
            })
        }
    }
    NetWorthResult {
        source: data.source,
        included_accounts: included.into_iter().collect(),
        rows,
    }
}
fn target_amount(t: &Target) -> Option<Money> {
    match t.kind {
        TargetKind::BalanceAmount { amount }
        | TargetKind::BalanceByDate { amount, .. }
        | TargetKind::FixedMonthlySavings { amount }
        | TargetKind::RefillToAmount { amount }
        | TargetKind::UpcomingExpense { amount, .. } => Some(amount),
        TargetKind::CreditCardPayoffByDate { .. } => None,
    }
}
#[must_use]
pub fn budget_progress(data: &ReportData<'_>, filter: &ReportFilter) -> BudgetProgressResult {
    let mut map: BTreeMap<(BudgetMonth, CategoryId), (Money, Money)> = BTreeMap::new();
    for a in data.assignments.iter().filter(|a| {
        filter.dates.contains(
            Date::from_calendar_date(
                a.month.year(),
                time::Month::try_from(a.month.month()).unwrap(),
                1,
            )
            .unwrap(),
        ) && category_ok(data, filter, a.category_id)
    }) {
        map.entry((a.month, a.category_id)).or_default().0 = map
            .get(&(a.month, a.category_id))
            .map_or(Money::ZERO, |x| x.0)
            .checked_add(a.amount)
            .unwrap()
    }
    for t in filtered(data, filter, true) {
        for (c, a) in lines(t) {
            if a < Money::ZERO && c.is_some_and(|x| category_ok(data, filter, x)) {
                let e = map.entry((month(t.date.0), c.unwrap())).or_default();
                e.1 = e.1.checked_add(a.checked_neg().unwrap()).unwrap()
            }
        }
    }
    let rows = map
        .into_iter()
        .map(|((month, category_id), (assigned, spent))| {
            let target = data
                .targets
                .iter()
                .find(|t| matches!(t.association,TargetAssociation::Category(c) if c==category_id))
                .and_then(target_amount);
            let underfunded = target.map_or(Money::ZERO, |x| {
                x.checked_sub(assigned).unwrap().max(Money::ZERO)
            });
            let available = assigned.checked_sub(spent).unwrap();
            let overspent = available
                .checked_neg()
                .unwrap_or(Money::ZERO)
                .max(Money::ZERO);
            let target_completion_basis_points = target.map(|x| {
                if x <= Money::ZERO {
                    10_000
                } else {
                    ((assigned.max(Money::ZERO).minor_units() as i128 * 10_000
                        / i128::from(x.minor_units()))
                    .clamp(0, 10_000)) as u16
                }
            });
            BudgetProgressRow {
                month,
                category_id,
                assigned,
                spent,
                target,
                target_completion_basis_points,
                underfunded,
                overspent,
            }
        })
        .collect();
    BudgetProgressResult {
        source: data.source,
        rows,
    }
}
#[must_use]
pub fn calculate(request: &ReportRequest, data: &ReportData<'_>) -> ReportResult {
    match request.kind {
        ReportKind::Spending => ReportResult::Spending(spending(data, &request.filter)),
        ReportKind::IncomeExpense => {
            ReportResult::IncomeExpense(income_expense(data, &request.filter))
        }
        ReportKind::NetWorth => ReportResult::NetWorth(net_worth(data, &request.filter)),
        ReportKind::BudgetProgress => {
            ReportResult::BudgetProgress(budget_progress(data, &request.filter))
        }
    }
}

/// Stable RFC-4180 CSV generated from the exact immutable result displayed by the UI.
pub fn export_csv(result: &ReportResult) -> Result<String, csv::Error> {
    let mut w = csv::WriterBuilder::new()
        .terminator(csv::Terminator::CRLF)
        .from_writer(vec![]);
    match result {
        ReportResult::Spending(r) => {
            w.write_record(["group_id", "category_id", "amount_usd"])?;
            for x in &r.rows {
                w.write_record([
                    x.group_id.to_string(),
                    x.category_id.to_string(),
                    usd(x.amount),
                ])?
            }
        }
        ReportResult::IncomeExpense(r) => {
            w.write_record(["month", "income_usd", "expense_usd", "net_usd"])?;
            for x in &r.rows {
                w.write_record([
                    format!("{:04}-{:02}", x.month.year(), x.month.month()),
                    usd(x.income),
                    usd(x.expense),
                    usd(x.net),
                ])?
            }
        }
        ReportResult::NetWorth(r) => {
            w.write_record(["date", "assets_usd", "liabilities_usd", "net_worth_usd"])?;
            for x in &r.rows {
                w.write_record([
                    x.date.to_string(),
                    usd(x.assets),
                    usd(x.liabilities),
                    usd(x.net_worth),
                ])?
            }
        }
        ReportResult::BudgetProgress(r) => {
            w.write_record([
                "month",
                "category_id",
                "assigned_usd",
                "spent_usd",
                "target_usd",
                "completion_basis_points",
                "underfunded_usd",
                "overspent_usd",
            ])?;
            for x in &r.rows {
                w.write_record([
                    format!("{:04}-{:02}", x.month.year(), x.month.month()),
                    x.category_id.to_string(),
                    usd(x.assigned),
                    usd(x.spent),
                    x.target.map_or_else(String::new, usd),
                    x.target_completion_basis_points
                        .map_or_else(String::new, |v| v.to_string()),
                    usd(x.underfunded),
                    usd(x.overspent),
                ])?
            }
        }
    }
    w.flush()?;
    Ok(String::from_utf8(w.into_inner().map_err(|e| e.into_error())?).expect("CSV is UTF-8"))
}
fn usd(v: Money) -> String {
    let n = i128::from(v.minor_units());
    format!(
        "{}{}.{:02}",
        if n < 0 { "-" } else { "" },
        n.abs() / 100,
        n.abs() % 100
    )
}
