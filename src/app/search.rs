//! Pure transaction-search parsing and safe query planning.
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
    Category(String),
    Payee(String),
    Memo(String),
    Amount {
        comparison: Comparison,
        value: Money,
    },
    Before(Date),
    After(Date),
    Cleared(bool),
    Approved(bool),
}
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct SearchAst {
    pub terms: Vec<SearchTerm>,
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
        "category" => Ok(SearchTerm::Category(value.into())),
        "payee" => Ok(SearchTerm::Payee(value.into())),
        "memo" => Ok(SearchTerm::Memo(value.into())),
        "before" => date(value).map(SearchTerm::Before),
        "after" => date(value).map(SearchTerm::After),
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
    let mut clauses = vec![];
    let mut binds = vec![];
    for t in &ast.terms {
        match t {
            SearchTerm::Text(v) => {
                clauses.push("(payees.name LIKE ? OR transactions.payee_snapshot LIKE ? OR transactions.memo LIKE ? OR categories.name LIKE ? OR accounts.name LIKE ? OR CAST(transactions.amount AS TEXT) LIKE ? OR transactions.transaction_date LIKE ?)".into());
                for _ in 0..7 {
                    binds.push(BindValue::Text(format!("%{v}%")))
                }
            }
            SearchTerm::Account(v)
            | SearchTerm::Category(v)
            | SearchTerm::Payee(v)
            | SearchTerm::Memo(v) => {
                let col = match t {
                    SearchTerm::Account(_) => "accounts.name",
                    SearchTerm::Category(_) => "categories.name",
                    SearchTerm::Payee(_) => "payees.name",
                    _ => "transactions.memo",
                };
                clauses.push(format!("{col} LIKE ?"));
                binds.push(BindValue::Text(format!("%{v}%")))
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

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SavedFilter {
    pub name: String,
    pub version: u32,
    pub filter: SearchAst,
    pub sort_order: i64,
    pub workspace: String,
    pub account_ids: Vec<AccountId>,
    pub category_ids: Vec<CategoryId>,
    pub payee_ids: Vec<PayeeId>,
}
impl SavedFilter {
    pub const VERSION: u32 = 1;
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
#[must_use]
pub fn useful_presets() -> Vec<SavedFilter> {
    [
        ("Unapproved", "approved:false"),
        ("Uncleared", "cleared:false"),
        ("Large outflows", "amount:<-100.00"),
    ]
    .into_iter()
    .filter_map(|(name, q)| {
        parse(q).ok().map(|filter| SavedFilter {
            name: name.into(),
            version: SavedFilter::VERSION,
            filter,
            sort_order: 0,
            workspace: "register".into(),
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
}
