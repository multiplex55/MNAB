//! Pure transaction-search parsing and safe query planning.
use crate::app::view_model::{RegisterScope, RegisterSortDirection, RegisterSortField};
use crate::domain::{AccountId, CategoryId, Money, PayeeId};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use time::Date;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum Comparison {
    Less,
    LessEqual,
    Equal,
    GreaterEqual,
    Greater,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SearchTerm {
    Text(String),
    Account(String),
    AccountGroup(String),
    Category(String),
    Payee(String),
    Memo(String),
    Amount {
        comparison: Comparison,
        value: Money,
    },
    Before(Date),
    After(Date),
    From(Date),
    Through(Date),
    Uncategorized(bool),
    Reconciled(bool),
    Imported(bool),
    Transfer(bool),
    Cleared(bool),
    Approved(bool),
}
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct SearchAst {
    pub terms: Vec<SearchTerm>,
}
impl SearchAst {
    /// Stable textual form used only at compatibility boundaries. Parsing it always
    /// returns this AST; UI surfaces must not implement their own parser.
    #[must_use]
    pub fn canonical_text(&self) -> String {
        self.terms
            .iter()
            .map(|term| match term {
                SearchTerm::Text(v) => quote(v),
                SearchTerm::Account(v) => format!("account:{}", quote(v)),
                SearchTerm::AccountGroup(v) => format!("group:{}", quote(v)),
                SearchTerm::Category(v) => format!("category:{}", quote(v)),
                SearchTerm::Payee(v) => format!("payee:{}", quote(v)),
                SearchTerm::Memo(v) => format!("memo:{}", quote(v)),
                SearchTerm::Amount { comparison, value } => format!(
                    "amount:{}{}",
                    match comparison {
                        Comparison::Less => "<",
                        Comparison::LessEqual => "<=",
                        Comparison::Equal => "=",
                        Comparison::GreaterEqual => ">=",
                        Comparison::Greater => ">",
                    },
                    value
                ),
                SearchTerm::Before(v) => format!("before:{v}"),
                SearchTerm::After(v) => format!("after:{v}"),
                SearchTerm::From(v) => format!("from:{v}"),
                SearchTerm::Through(v) => format!("through:{v}"),
                SearchTerm::Uncategorized(v) => format!("uncategorized:{v}"),
                SearchTerm::Reconciled(v) => format!("reconciled:{v}"),
                SearchTerm::Imported(v) => format!("imported:{v}"),
                SearchTerm::Transfer(v) => format!("transfer:{v}"),
                SearchTerm::Cleared(v) => format!("cleared:{v}"),
                SearchTerm::Approved(v) => format!("approved:{v}"),
            })
            .collect::<Vec<_>>()
            .join(" ")
    }
}
fn quote(value: &str) -> String {
    if value.chars().any(char::is_whitespace) || value.contains(['"', '\\']) {
        format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        value.to_owned()
    }
}

#[must_use]
pub fn register_filter(ast: &SearchAst) -> crate::app::view_model::RegisterFilter {
    crate::app::view_model::RegisterFilter {
        search: ast.canonical_text(),
        ..Default::default()
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DiagnosticKind {
    UnknownField,
    MissingValue,
    InvalidAmount,
    InvalidDate,
    InvalidBoolean,
    UnterminatedQuote,
    InvalidEscape,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchDiagnostic {
    pub kind: DiagnosticKind,
    pub span: Span,
    pub message: String,
}

pub fn parse(input: &str) -> Result<SearchAst, Vec<SearchDiagnostic>> {
    let mut raw = vec![];
    let mut errors = vec![];
    let b = input.as_bytes();
    let mut i = 0;
    while i < b.len() {
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1
        }
        if i == b.len() {
            break;
        }
        let start = i;
        let mut s = String::new();
        let mut quoted = false;
        while i < b.len() && (!b[i].is_ascii_whitespace() || quoted) {
            match b[i] {
                b'"' => {
                    quoted = !quoted;
                    i += 1
                }
                b'\\' if quoted => {
                    i += 1;
                    if i >= b.len() || !matches!(b[i], b'"' | b'\\') {
                        errors.push(SearchDiagnostic {
                            kind: DiagnosticKind::InvalidEscape,
                            span: Span {
                                start,
                                end: i.saturating_add(1),
                            },
                            message: r#"only \" and \\ may be escaped"#.into(),
                        });
                        break;
                    }
                    s.push(char::from(b[i]));
                    i += 1
                }
                _ => {
                    let c = input[i..].chars().next().unwrap();
                    s.push(c);
                    i += c.len_utf8()
                }
            }
        }
        if quoted {
            errors.push(SearchDiagnostic {
                kind: DiagnosticKind::UnterminatedQuote,
                span: Span { start, end: i },
                message: "unterminated quoted string".into(),
            })
        }
        raw.push((s, Span { start, end: i }));
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    let mut terms = vec![];
    for (s, span) in raw {
        match term(&s) {
            Ok(x) => terms.push(x),
            Err((kind, msg)) => errors.push(SearchDiagnostic {
                kind,
                span,
                message: msg,
            }),
        }
    }
    if errors.is_empty() {
        Ok(SearchAst { terms })
    } else {
        Err(errors)
    }
}
fn term(s: &str) -> Result<SearchTerm, (DiagnosticKind, String)> {
    let Some((field, value)) = s.split_once(':') else {
        return Ok(SearchTerm::Text(s.into()));
    };
    if value.is_empty() {
        return Err((
            DiagnosticKind::MissingValue,
            format!("{field} requires a value"),
        ));
    }
    match field.to_ascii_lowercase().as_str() {
        "account" => Ok(SearchTerm::Account(value.into())),
        "group" | "account-group" => Ok(SearchTerm::AccountGroup(value.into())),
        "category" => Ok(SearchTerm::Category(value.into())),
        "payee" => Ok(SearchTerm::Payee(value.into())),
        "memo" => Ok(SearchTerm::Memo(value.into())),
        "before" => date(value).map(SearchTerm::Before),
        "after" => date(value).map(SearchTerm::After),
        "from" => date(value).map(SearchTerm::From),
        "through" => date(value).map(SearchTerm::Through),
        "uncategorized" | "category-none" => boolean(value).map(SearchTerm::Uncategorized),
        "reconciled" => boolean(value).map(SearchTerm::Reconciled),
        "imported" => boolean(value).map(SearchTerm::Imported),
        "transfer" | "transfers" => boolean(value).map(SearchTerm::Transfer),
        "cleared" => boolean(value).map(SearchTerm::Cleared),
        "approved" => boolean(value).map(SearchTerm::Approved),
        "amount" => amount(value),
        _ => Err((
            DiagnosticKind::UnknownField,
            format!("unknown search field `{field}`"),
        )),
    }
}
fn date(v: &str) -> Result<Date, (DiagnosticKind, String)> {
    Date::parse(v, &time::format_description::well_known::Iso8601::DATE).map_err(|_| {
        (
            DiagnosticKind::InvalidDate,
            "expected an ISO date (YYYY-MM-DD)".into(),
        )
    })
}
fn boolean(v: &str) -> Result<bool, (DiagnosticKind, String)> {
    match v.to_ascii_lowercase().as_str() {
        "true" | "yes" | "1" => Ok(true),
        "false" | "no" | "0" => Ok(false),
        _ => Err((
            DiagnosticKind::InvalidBoolean,
            "expected true or false".into(),
        )),
    }
}
fn amount(v: &str) -> Result<SearchTerm, (DiagnosticKind, String)> {
    let (comparison, n) = if let Some(x) = v.strip_prefix(">=") {
        (Comparison::GreaterEqual, x)
    } else if let Some(x) = v.strip_prefix("<=") {
        (Comparison::LessEqual, x)
    } else if let Some(x) = v.strip_prefix('>') {
        (Comparison::Greater, x)
    } else if let Some(x) = v.strip_prefix('<') {
        (Comparison::Less, x)
    } else if let Some(x) = v.strip_prefix('=') {
        (Comparison::Equal, x)
    } else {
        (Comparison::Equal, v)
    };
    n.parse()
        .map(|value| SearchTerm::Amount { comparison, value })
        .map_err(|_| (DiagnosticKind::InvalidAmount, "invalid USD amount".into()))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BindValue {
    Text(String),
    Integer(i64),
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct QueryPlan {
    pub where_sql: String,
    pub binds: Vec<BindValue>,
}
/// Produces SQL containing placeholders only; user text is always returned as a bind value.
#[must_use]
pub fn compile(ast: &SearchAst) -> QueryPlan {
    fn like(v: &str) -> String {
        format!(
            "%{}%",
            v.replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_")
        )
    }
    let mut clauses = vec![];
    let mut binds = vec![];
    for t in &ast.terms {
        match t {
            SearchTerm::Text(v) => {
                clauses.push("(payees.name LIKE ? ESCAPE '\\' OR transactions.payee_snapshot LIKE ? ESCAPE '\\' OR transactions.memo LIKE ? ESCAPE '\\' OR categories.name LIKE ? ESCAPE '\\' OR accounts.name LIKE ? ESCAPE '\\' OR CAST(transactions.amount AS TEXT) LIKE ? ESCAPE '\\' OR transactions.transaction_date LIKE ? ESCAPE '\\')".into());
                for _ in 0..7 {
                    binds.push(BindValue::Text(like(v)))
                }
            }
            SearchTerm::Account(v)
            | SearchTerm::AccountGroup(v)
            | SearchTerm::Category(v)
            | SearchTerm::Payee(v)
            | SearchTerm::Memo(v) => {
                let col = match t {
                    SearchTerm::Account(_) => "accounts.name",
                    SearchTerm::AccountGroup(_) => "account_groups.name",
                    SearchTerm::Category(_) => "categories.name",
                    SearchTerm::Payee(_) => "payees.name",
                    _ => "transactions.memo",
                };
                clauses.push(format!("{col} LIKE ? ESCAPE '\\\\'"));
                binds.push(BindValue::Text(like(v)))
            }
            SearchTerm::Amount { comparison, value } => {
                let op = match comparison {
                    Comparison::Less => "<",
                    Comparison::LessEqual => "<=",
                    Comparison::Equal => "=",
                    Comparison::GreaterEqual => ">=",
                    Comparison::Greater => ">",
                };
                clauses.push(format!("transactions.amount {op} ?"));
                binds.push(BindValue::Integer(value.minor_units()))
            }
            SearchTerm::Before(v) | SearchTerm::After(v) => {
                clauses.push(format!(
                    "transactions.transaction_date {} ?",
                    if matches!(t, SearchTerm::Before(_)) {
                        "<"
                    } else {
                        " >"
                    }
                ));
                binds.push(BindValue::Text(v.to_string()))
            }
            SearchTerm::From(v) | SearchTerm::Through(v) => {
                clauses.push(format!(
                    "transactions.transaction_date {} ?",
                    if matches!(t, SearchTerm::From(_)) {
                        ">="
                    } else {
                        "<="
                    }
                ));
                binds.push(BindValue::Text(v.to_string()));
            }
            SearchTerm::Uncategorized(v) => clauses.push(format!(
                "transactions.category_id IS {}NULL",
                if *v { "" } else { "NOT " }
            )),
            SearchTerm::Reconciled(v) => {
                clauses.push(format!(
                    "transactions.cleared_state {} ?",
                    if *v { "=" } else { "<>" }
                ));
                binds.push(BindValue::Text("reconciled".into()));
            }
            SearchTerm::Imported(v) => clauses.push(format!(
                "transactions.import_batch_id IS {}NULL",
                if *v { "NOT " } else { "" }
            )),
            SearchTerm::Transfer(v) => clauses.push(format!(
                "transactions.transfer_id IS {}NULL",
                if *v { "NOT " } else { "" }
            )),
            SearchTerm::Cleared(v) => {
                clauses.push(if *v {
                    "transactions.cleared_state <> ?".into()
                } else {
                    "transactions.cleared_state = ?".into()
                });
                binds.push(BindValue::Text("uncleared".into()))
            }
            SearchTerm::Approved(v) => {
                clauses.push("transactions.approval_state = ?".into());
                binds.push(BindValue::Text(
                    if *v { "approved" } else { "unapproved" }.into(),
                ))
            }
        }
    }
    QueryPlan {
        where_sql: clauses.join(" AND "),
        binds,
    }
}

/// Complete, durable register presentation.  Deliberately contains no cursor,
/// selection, editor, error, page, or other session state.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SavedView {
    pub name: String,
    pub version: u32,
    pub scope: RegisterScope,
    pub filter: SearchAst,
    pub sort: RegisterSort,
    pub columns: Option<SavedColumns>,
    pub account_ids: Vec<AccountId>,
    pub category_ids: Vec<CategoryId>,
    pub payee_ids: Vec<PayeeId>,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RegisterSort {
    pub field: RegisterSortField,
    pub direction: RegisterSortDirection,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SavedColumns {
    Named { name: String },
    Visible { visible: Vec<String> },
}

impl SavedView {
    pub const VERSION: u32 = 2;
    #[must_use]
    pub fn missing_references(
        &self,
        accounts: &[AccountId],
        categories: &[CategoryId],
        payees: &[PayeeId],
    ) -> Vec<String> {
        self.account_ids
            .iter()
            .filter(|x| !accounts.contains(x))
            .map(|x| format!("Missing or archived account {x}"))
            .chain(
                self.category_ids
                    .iter()
                    .filter(|x| !categories.contains(x))
                    .map(|x| format!("Missing or archived category {x}")),
            )
            .chain(
                self.payee_ids
                    .iter()
                    .filter(|x| !payees.contains(x))
                    .map(|x| format!("Missing or archived payee {x}")),
            )
            .collect()
    }
}

impl<'de> Deserialize<'de> for SavedView {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::Error as _;
        let value = serde_json::Value::deserialize(deserializer)?;
        let version = value
            .get("version")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| D::Error::custom("saved view version is missing or malformed"))?;
        if version > u64::from(Self::VERSION) {
            return Err(D::Error::custom(format!(
                "unsupported saved view version {version}"
            )));
        }
        if version == 0 {
            return Err(D::Error::custom("saved view version 0 is malformed"));
        }
        if version == 1 {
            #[derive(Deserialize)]
            struct Legacy {
                name: String,
                filter: SearchAst,
                #[serde(default)]
                sort_order: i64,
                workspace: String,
                #[serde(default)]
                account_ids: Vec<AccountId>,
                #[serde(default)]
                category_ids: Vec<CategoryId>,
                #[serde(default)]
                payee_ids: Vec<PayeeId>,
            }
            let old: Legacy = serde_json::from_value(value).map_err(D::Error::custom)?;
            let scope = legacy_scope(&old.workspace, &old.account_ids).map_err(D::Error::custom)?;
            return Ok(Self {
                name: old.name,
                version: Self::VERSION,
                scope,
                filter: old.filter,
                sort: RegisterSort {
                    field: RegisterSortField::Date,
                    direction: if old.sort_order < 0 {
                        RegisterSortDirection::Descending
                    } else {
                        RegisterSortDirection::Ascending
                    },
                },
                columns: None,
                account_ids: old.account_ids,
                category_ids: old.category_ids,
                payee_ids: old.payee_ids,
            });
        }
        #[derive(Deserialize)]
        struct Current {
            name: String,
            scope: RegisterScope,
            filter: SearchAst,
            sort: RegisterSort,
            #[serde(default)]
            columns: Option<SavedColumns>,
            #[serde(default)]
            account_ids: Vec<AccountId>,
            #[serde(default)]
            category_ids: Vec<CategoryId>,
            #[serde(default)]
            payee_ids: Vec<PayeeId>,
        }
        let current: Current = serde_json::from_value(value).map_err(D::Error::custom)?;
        Ok(Self {
            name: current.name,
            version: Self::VERSION,
            scope: current.scope,
            filter: current.filter,
            sort: current.sort,
            columns: current.columns,
            account_ids: current.account_ids,
            category_ids: current.category_ids,
            payee_ids: current.payee_ids,
        })
    }
}

fn legacy_scope(workspace: &str, accounts: &[AccountId]) -> Result<RegisterScope, String> {
    let normalized = workspace.trim().to_ascii_lowercase();
    if matches!(
        normalized.as_str(),
        "register" | "all" | "all-transactions" | "all_transactions"
    ) {
        return Ok(RegisterScope::AllTransactions);
    }
    if normalized == "account" {
        return accounts
            .first()
            .copied()
            .map(RegisterScope::Account)
            .ok_or_else(|| "legacy account view has no account id".into());
    }
    if let Some(raw) = normalized.strip_prefix("account:") {
        return raw
            .parse()
            .map(RegisterScope::Account)
            .map_err(|_| "legacy account workspace contains an invalid account id".into());
    }
    Err(format!("unsupported legacy workspace `{workspace}`"))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QuickFilter {
    Unapproved,
    Uncategorized,
    Uncleared,
    Imported,
    Transfers,
}

impl QuickFilter {
    const fn term(self) -> SearchTerm {
        match self {
            Self::Unapproved => SearchTerm::Approved(false),
            Self::Uncategorized => SearchTerm::Uncategorized(true),
            Self::Uncleared => SearchTerm::Cleared(false),
            Self::Imported => SearchTerm::Imported(true),
            Self::Transfers => SearchTerm::Transfer(true),
        }
    }
    /// Toggles this chip in the same AST submitted to the worker.
    pub fn toggle(self, ast: &mut SearchAst) {
        let term = self.term();
        if let Some(position) = ast.terms.iter().position(|candidate| *candidate == term) {
            ast.terms.remove(position);
        } else {
            ast.terms.push(term);
        }
    }
}
#[must_use]
pub fn useful_presets() -> Vec<SavedView> {
    [
        ("Unapproved", "approved:false"),
        ("Uncategorized", "uncategorized:true"),
        ("Uncleared", "cleared:false"),
        ("Imported", "imported:true"),
        ("Transfers", "transfer:true"),
        ("Needs Review", "approved:false"),
        ("Recent Imports", "imported:true approved:false"),
        ("Large outflows", "amount:<-100.00"),
    ]
    .into_iter()
    .filter_map(|(name, q)| {
        parse(q).ok().map(|filter| SavedView {
            name: name.into(),
            version: SavedView::VERSION,
            scope: RegisterScope::AllTransactions,
            filter,
            sort: RegisterSort {
                field: RegisterSortField::Date,
                direction: RegisterSortDirection::Descending,
            },
            columns: None,
            account_ids: vec![],
            category_ids: vec![],
            payee_ids: vec![],
        })
    })
    .collect()
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum ResultSection {
    Transactions,
    Accounts,
    Categories,
    Payees,
    Commands,
    Settings,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NavigationTarget {
    RegisterTransaction {
        account_id: AccountId,
        transaction_id: crate::domain::TransactionId,
    },
    BudgetCategory {
        category_id: CategoryId,
        month: crate::domain::BudgetMonth,
    },
    Account(AccountId),
    Command(crate::app::command::AppCommand),
    SettingsControl(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchResult {
    pub title: String,
    pub target: NavigationTarget,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResultGroup {
    pub section: ResultSection,
    pub items: Vec<SearchResult>,
    pub continuation: Option<usize>,
}

/// Groups independently bounded sections. `continuation` is the next per-section
/// offset, so expanding one kind never turns the complete result set unbounded.
#[must_use]
pub fn bounded_groups(
    sections: impl IntoIterator<Item = (ResultSection, Vec<SearchResult>)>,
    limit: usize,
) -> Vec<ResultGroup> {
    let limit = limit.max(1);
    sections
        .into_iter()
        .map(|(section, mut items)| {
            let continuation = (items.len() > limit).then_some(limit);
            items.truncate(limit);
            ResultGroup {
                section,
                items,
                continuation,
            }
        })
        .collect()
}

#[derive(Clone, Debug)]
pub struct SearchDebounce {
    delay: Duration,
    changed_at: Option<Instant>,
    pending: Option<String>,
}
impl SearchDebounce {
    #[must_use]
    pub fn new(delay: Duration) -> Self {
        Self {
            delay,
            changed_at: None,
            pending: None,
        }
    }
    pub fn update(&mut self, text: impl Into<String>, now: Instant) {
        self.pending = Some(text.into());
        self.changed_at = Some(now);
    }
    pub fn take_ready(&mut self, now: Instant) -> Option<String> {
        if now.duration_since(self.changed_at?) < self.delay {
            return None;
        }
        self.changed_at = None;
        self.pending.take()
    }
}

#[cfg(test)]
mod result_tests {
    use super::*;

    #[test]
    fn groups_are_independently_bounded() {
        let account = AccountId::new();
        let item = || SearchResult {
            title: "Account".into(),
            target: NavigationTarget::Account(account),
        };
        let groups = bounded_groups(
            [
                (ResultSection::Accounts, vec![item(), item(), item()]),
                (ResultSection::Settings, vec![item()]),
            ],
            2,
        );
        assert_eq!(groups[0].items.len(), 2);
        assert_eq!(groups[0].continuation, Some(2));
        assert_eq!(groups[1].continuation, None);
    }

    #[test]
    fn text_search_is_debounced() {
        let start = Instant::now();
        let mut debounce = SearchDebounce::new(Duration::from_millis(200));
        debounce.update("rent", start);
        assert_eq!(
            debounce.take_ready(start + Duration::from_millis(199)),
            None
        );
        assert_eq!(
            debounce.take_ready(start + Duration::from_millis(200)),
            Some("rent".into())
        );
    }

    #[test]
    fn parser_fields_become_parameterized_query_bindings() {
        let ast = parse("account:Checking category:Food payee:Market memo:weekly amount:>=12.34 after:2026-01-01 before:2026-12-31 cleared:true approved:false").unwrap();
        let plan = compile(&ast);
        assert!(plan.where_sql.contains("accounts.name LIKE ?"));
        assert!(plan.where_sql.contains("transactions.amount >= ?"));
        assert!(plan.where_sql.contains("transactions.cleared_state <> ?"));
        assert_eq!(plan.binds[4], BindValue::Integer(1234));
        assert_eq!(
            plan.binds.last(),
            Some(&BindValue::Text("unapproved".into()))
        );
        assert!(!plan.where_sql.contains("Checking"));
    }

    #[test]
    fn parser_handles_quotes_escapes_unicode_and_repeated_fields() {
        let ast = parse(r#"coffee payee:"Blue \"Bottle\"" memo:"emoji ☕ \\ note" category:Food category:Dining"#).unwrap();
        assert_eq!(ast.terms.len(), 5);
        assert!(matches!(&ast.terms[1], SearchTerm::Payee(v) if v == "Blue \"Bottle\""));
        assert!(matches!(&ast.terms[2], SearchTerm::Memo(v) if v == "emoji ☕ \\ note"));
        assert!(matches!(&ast.terms[3], SearchTerm::Category(v) if v == "Food"));
        assert!(matches!(&ast.terms[4], SearchTerm::Category(v) if v == "Dining"));
    }

    #[test]
    fn parser_reports_safe_errors_for_bad_literals_and_malicious_text() {
        for (input, kind) in [
            ("before:2026-99-99", DiagnosticKind::InvalidDate),
            ("amount:abc", DiagnosticKind::InvalidAmount),
            ("cleared:maybe", DiagnosticKind::InvalidBoolean),
            ("unknown:x", DiagnosticKind::UnknownField),
            ("memo:", DiagnosticKind::MissingValue),
            (r#"payee:"unterminated"#, DiagnosticKind::UnterminatedQuote),
            (r#"payee:"bad\q""#, DiagnosticKind::InvalidEscape),
        ] {
            let errors = parse(input).unwrap_err();
            assert_eq!(errors[0].kind, kind);
            assert!(!errors[0].message.contains(input));
        }
        let ast = parse(r#"memo:"x' OR 1=1 --""#).unwrap();
        let plan = compile(&ast);
        assert!(!plan.where_sql.contains("OR 1=1"));
        assert!(
            plan.binds
                .iter()
                .any(|v| matches!(v, BindValue::Text(t) if t.contains("OR 1=1")))
        );
    }

    #[test]
    fn saved_view_migrates_and_round_trips_without_transient_state() {
        let account = AccountId::new();
        let legacy = serde_json::json!({"name":"Old","version":1,"filter":{"terms":[{"Approved":false}]},
            "sort_order":-1,"workspace":"account","account_ids":[account],"category_ids":[],"payee_ids":[]});
        let migrated: SavedView = serde_json::from_value(legacy).unwrap();
        assert_eq!(migrated.scope, RegisterScope::Account(account));
        assert_eq!(migrated.version, SavedView::VERSION);
        let json = serde_json::to_string(&migrated).unwrap();
        assert!(
            !json.contains("selection") && !json.contains("cursor") && !json.contains("editor")
        );
        assert_eq!(serde_json::from_str::<SavedView>(&json).unwrap(), migrated);
        assert!(serde_json::from_value::<SavedView>(serde_json::json!({"version":99})).is_err());
        assert!(serde_json::from_value::<SavedView>(serde_json::json!({"version":"two"})).is_err());
    }

    #[test]
    fn quick_filters_are_canonical_and_worker_ready() {
        for (chip, expected) in [
            (QuickFilter::Unapproved, SearchTerm::Approved(false)),
            (QuickFilter::Uncategorized, SearchTerm::Uncategorized(true)),
            (QuickFilter::Uncleared, SearchTerm::Cleared(false)),
            (QuickFilter::Imported, SearchTerm::Imported(true)),
            (QuickFilter::Transfers, SearchTerm::Transfer(true)),
        ] {
            let mut ast = SearchAst::default();
            chip.toggle(&mut ast);
            assert_eq!(ast.terms, vec![expected]);
            assert_eq!(parse(&register_filter(&ast).search).unwrap(), ast);
        }
    }

    #[test]
    fn missing_reference_diagnostics_survive_migration() {
        let missing = AccountId::new();
        let view = SavedView {
            name: "x".into(),
            version: SavedView::VERSION,
            scope: RegisterScope::AllTransactions,
            filter: SearchAst::default(),
            sort: RegisterSort {
                field: RegisterSortField::Date,
                direction: RegisterSortDirection::Descending,
            },
            columns: None,
            account_ids: vec![missing],
            category_ids: vec![],
            payee_ids: vec![],
        };
        assert_eq!(
            view.missing_references(&[], &[], &[]),
            vec![format!("Missing or archived account {missing}")]
        );
    }
}
