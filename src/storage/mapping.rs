//! The single conversion boundary between SQLite primitives and validated domain values.
use super::{model::*, repository::RowConversionError};
use crate::domain::*;
use time::{Date, format_description::well_known::Iso8601};
use uuid::Uuid;

/// Converts a legacy merchant row into the generalized representation without changing its
/// exact/contains/prefix behavior. Regex survives only as an isolated non-authorable condition.
pub fn legacy_transaction_rule(
    row: &MerchantRuleRow,
) -> Result<TransactionRule, crate::error::RepositoryError> {
    let id = row
        .id
        .parse()
        .map(TransactionRuleId::from_uuid)
        .map_err(projection_error)?;
    let account = row
        .account_id
        .as_deref()
        .map(str::parse)
        .transpose()
        .map_err(projection_error)?;
    let payee = row
        .payee_id
        .as_deref()
        .map(str::parse)
        .transpose()
        .map_err(projection_error)?;
    let category = row
        .category_id
        .as_deref()
        .map(str::parse)
        .transpose()
        .map_err(projection_error)?;
    let merchant = match row.match_type.as_str() {
        "exact" => RuleCondition::Merchant {
            value: normalize_merchant(&row.pattern),
            match_type: TextMatch::Exact,
        },
        "contains" => RuleCondition::Merchant {
            value: normalize_merchant(&row.pattern),
            match_type: TextMatch::Contains,
        },
        "prefix" => RuleCondition::Merchant {
            value: normalize_merchant(&row.pattern),
            match_type: TextMatch::Prefix,
        },
        "regex" => RuleCondition::LegacyMerchantRegex {
            pattern: row.pattern.clone(),
        },
        other => {
            return Err(projection_error(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("unknown merchant match type {other}"),
            )));
        }
    };
    let mut conditions = vec![merchant];
    if let Some(account) = account {
        conditions.push(RuleCondition::Account(account));
    }
    let mut actions = Vec::new();
    if let Some(payee_id) = payee {
        actions.push(RuleAction::SetPayee {
            payee_id,
            display_name_snapshot: String::new(),
        });
    }
    if let Some(category_id) = category {
        actions.push(RuleAction::SetCategory { category_id });
    }
    Ok(TransactionRule {
        id,
        name: format!("Merchant: {}", row.pattern),
        description: "Migrated merchant rule".into(),
        enabled: row.enabled != 0,
        priority: i32::try_from(row.priority).unwrap_or(if row.priority < 0 {
            i32::MIN
        } else {
            i32::MAX
        }),
        origin: MerchantRuleOrigin::Explicit,
        conditions,
        actions,
        confidence: MerchantConfidence::High,
        usage_count: 0,
        match_count: 0,
        last_used_date: None,
    })
}

fn projection_error(
    error: impl std::error::Error + Send + Sync + 'static,
) -> crate::error::RepositoryError {
    crate::error::RepositoryError::Failed {
        source: Box::new(error),
    }
}

#[must_use]
pub const fn version(
    generation: crate::storage::worker::Generation,
    revision: u64,
) -> crate::app::view_model::ViewVersion {
    crate::app::view_model::ViewVersion {
        generation: generation.view,
        revision,
    }
}

#[must_use]
pub fn outcome(
    summary: &str,
    generation: crate::storage::worker::Generation,
) -> crate::app::view_model::CommandOutcomeView {
    crate::app::view_model::CommandOutcomeView {
        version: version(generation, 0),
        summary: summary.into(),
    }
}

pub fn register_page(
    page: crate::storage::query_store::RegisterPage,
    request: crate::app::view_model::RegisterRequest,
    generation: crate::storage::worker::Generation,
) -> Result<crate::app::view_model::RegisterPageView, crate::error::RepositoryError> {
    use crate::app::view_model::*;
    let rows = page
        .rows
        .into_iter()
        .map(|row| {
            Ok(RegisterRowView {
                transaction_id: TransactionId::from_uuid(
                    uuid(&row.transaction_id, "transactions", &row.transaction_id)
                        .map_err(projection_error)?,
                ),
                account_id: AccountId::from_uuid(
                    uuid(&row.account_id, "accounts", &row.account_id).map_err(projection_error)?,
                ),
                account_name: row.account_name,
                date: date(&row.date, "transactions", &row.transaction_id)
                    .map_err(projection_error)?,
                created_at: row.created_at,
                payee_id: row
                    .payee_id
                    .as_deref()
                    .map(|id| {
                        uuid(id, "payees", id)
                            .map(PayeeId::from_uuid)
                            .map_err(projection_error)
                    })
                    .transpose()?,
                payee_name: row.payee,
                category_id: row
                    .category_id
                    .as_deref()
                    .map(|id| {
                        uuid(id, "categories", id)
                            .map(CategoryId::from_uuid)
                            .map_err(projection_error)
                    })
                    .transpose()?,
                category_name: row.category,
                memo: row.memo,
                inflow_cents: row.amount.minor_units().max(0),
                outflow_cents: row.amount.minor_units().saturating_neg().max(0),
                cleared_state: row.cleared_state.clone(),
                approved: row.approval_state == "approved",
                reconciled: row.cleared_state == "reconciled",
                is_transfer: row.transfer_id.is_some(),
                transfer_id: row.transfer_id,
                split_count: row.split_count,
                import_batch_id: row
                    .import_batch_id
                    .as_deref()
                    .map(|id| {
                        uuid(id, "import_batches", id)
                            .map(ImportBatchId::from_uuid)
                            .map_err(projection_error)
                    })
                    .transpose()?,
                import_source: row.import_source,
                review_state: row.review_state,
                running_balance_cents: row.running_balance.map(|m| m.minor_units()),
            })
        })
        .collect::<Result<Vec<_>, crate::error::RepositoryError>>()?;
    let next_cursor = page
        .next_cursor
        .map(|cursor| -> Result<_, crate::error::RepositoryError> {
            Ok(RegisterCursor {
                date: cursor.date,
                created_at: cursor.created_at,
                transaction_id: cursor.transaction_id,
            })
        })
        .transpose()?;
    Ok(RegisterPageView {
        version: version(generation, 0),
        scope: request.scope,
        cursor: request.cursor.clone(),
        request,
        next_cursor,
        total_matches: page.total_matches,
        has_more: page.has_more,
        rows,
        separators: vec![],
    })
}

pub fn search_results(
    query: &str,
    rows: Vec<crate::storage::query_store::SearchRow>,
    generation: crate::storage::worker::Generation,
) -> Result<crate::app::view_model::SearchResultsView, crate::error::RepositoryError> {
    use crate::app::view_model::{DisplayMoney, HighlightSpanView, SearchResultItemView};
    let results = rows
        .into_iter()
        .map(|row| {
            let transaction_id = TransactionId::from_uuid(
                uuid(&row.transaction_id, "transactions", &row.transaction_id)
                    .map_err(projection_error)?,
            );
            let account_id = AccountId::from_uuid(
                uuid(&row.account_id, "accounts", &row.account_id).map_err(projection_error)?,
            );
            let parsed_date =
                date(&row.date, "transactions", &row.transaction_id).map_err(projection_error)?;
            Ok(SearchResultItemView {
                transaction_id,
                account_id,
                account: row.account.clone(),
                date: parsed_date,
                payee: row.payee.clone(),
                category: row.category.clone(),
                memo: row.memo.clone(),
                amount: DisplayMoney::usd(row.amount.minor_units()),
                approved: row.approved,
                clearance: row.clearance.clone(),
                title: row.payee,
                subtitle: format!("{} · {} · {}", row.date, row.account, row.category),
                highlights: row
                    .highlights
                    .into_iter()
                    .map(|span| HighlightSpanView {
                        field: span.field,
                        start: span.start,
                        end: span.end,
                    })
                    .collect(),
            })
        })
        .collect::<Result<Vec<_>, crate::error::RepositoryError>>()?;
    Ok(crate::app::view_model::SearchResultsView {
        version: version(generation, 0),
        query: query.into(),
        metadata: crate::app::view_model::SortFilterMetadata {
            sort_key: "relevance".into(),
            filter_summary: query.into(),
            ..Default::default()
        },
        results,
    })
}

#[must_use]
pub fn diagnostics(
    findings: Vec<crate::storage::diagnostics::Finding>,
    generation: crate::storage::worker::Generation,
) -> crate::app::view_model::DiagnosticsView {
    crate::app::view_model::DiagnosticsView {
        version: version(generation, 0),
        findings: findings
            .into_iter()
            .map(|f| crate::app::view_model::DiagnosticFindingView {
                severity: format!("{:?}", f.severity).to_lowercase(),
                check: f.check,
                entity_reference: f.entity_id,
                safe_explanation: f.summary,
                safe_remediation: f.remediation,
            })
            .collect(),
    }
}

pub fn occurrences(
    _connection: &rusqlite::Connection,
    through: Date,
    generation: crate::storage::worker::Generation,
) -> Result<crate::app::view_model::OccurrencesView, rusqlite::Error> {
    Ok(crate::app::view_model::OccurrencesView {
        version: version(generation, 0),
        through,
        occurrences: vec![],
    })
}

#[must_use]
pub fn report_view(
    result: ReportResult,
    generation: crate::storage::worker::Generation,
) -> crate::app::view_model::ReportView {
    use crate::app::view_model::{ReportPointView, ReportView};
    let csv = crate::domain::export_csv(&result).unwrap_or_default();
    let (revision, title, points, total) = match result {
        ReportResult::IncomeExpense(v) => (
            v.source.revision,
            "Income and expense",
            v.rows
                .into_iter()
                .map(|r| ReportPointView {
                    label: format!("{:04}-{:02}", r.month.year(), r.month.month()),
                    income_cents: r.income.minor_units(),
                    expense_cents: r.expense.minor_units(),
                    net_cents: r.net.minor_units(),
                })
                .collect(),
            v.net.minor_units(),
        ),
        ReportResult::NetWorth(v) => (
            v.source.revision,
            "Net worth",
            v.rows
                .into_iter()
                .map(|r| ReportPointView {
                    label: r.date.to_string(),
                    income_cents: 0,
                    expense_cents: 0,
                    net_cents: r.net_worth.minor_units(),
                })
                .collect(),
            v.total.minor_units(),
        ),
        ReportResult::Spending(v) => (
            v.source.revision,
            "Spending",
            v.monthly
                .into_iter()
                .map(|r| ReportPointView {
                    label: format!("{:04}-{:02}", r.month.year(), r.month.month()),
                    income_cents: 0,
                    expense_cents: r.amount.minor_units(),
                    net_cents: -r.amount.minor_units(),
                })
                .collect(),
            v.total.minor_units(),
        ),
        ReportResult::BudgetProgress(v) => (
            v.source.revision,
            "Budget progress",
            vec![],
            v.total_assigned.minor_units() - v.total_spent.minor_units(),
        ),
    };
    ReportView {
        version: version(generation, revision),
        title: title.into(),
        points,
        total_cents: total,
        csv,
    }
}

fn bad(table: &'static str, id: &str, reason: &'static str) -> RowConversionError {
    RowConversionError::new(table, id, reason)
}
pub fn uuid(text: &str, table: &'static str, id: &str) -> Result<Uuid, RowConversionError> {
    Uuid::parse_str(text).map_err(|_| bad(table, id, "invalid identifier"))
}
pub fn date(text: &str, table: &'static str, id: &str) -> Result<Date, RowConversionError> {
    Date::parse(text, &Iso8601::DATE).map_err(|_| bad(table, id, "invalid date"))
}
pub const fn money(cents: i64) -> Money {
    Money::from_minor_units(cents)
}
fn boolean(value: i64, table: &'static str, id: &str) -> Result<bool, RowConversionError> {
    match value {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(bad(table, id, "invalid boolean")),
    }
}
pub fn account_type(value: &str, id: &str) -> Result<AccountType, RowConversionError> {
    match value {
        "checking" => Ok(AccountType::Checking),
        "savings" => Ok(AccountType::Savings),
        "cash" => Ok(AccountType::Cash),
        "credit_card" => Ok(AccountType::CreditCard),
        "loan" => Ok(AccountType::Loan),
        "asset" => Ok(AccountType::Asset),
        "liability" => Ok(AccountType::Liability),
        "investment" => Err(bad(
            "accounts",
            id,
            "investment account type is no longer supported",
        )),
        _ => Err(bad("accounts", id, "invalid account type")),
    }
}
pub fn clearance(value: &str, id: &str) -> Result<Clearance, RowConversionError> {
    match value {
        "uncleared" => Ok(Clearance::Uncleared),
        "cleared" => Ok(Clearance::Cleared),
        "reconciled" => Ok(Clearance::Reconciled),
        _ => Err(bad("transactions", id, "invalid clearance")),
    }
}
pub fn approval(value: &str, id: &str) -> Result<Approval, RowConversionError> {
    match value {
        "unapproved" => Ok(Approval::Unapproved),
        "approved" => Ok(Approval::Approved),
        _ => Err(bad("transactions", id, "invalid approval")),
    }
}

impl TryFrom<BudgetRow> for Budget {
    type Error = RowConversionError;
    fn try_from(r: BudgetRow) -> Result<Self, Self::Error> {
        Ok(Self {
            id: BudgetId::from_uuid(uuid(&r.id, "budgets", &r.id)?),
            name: r.name,
        })
    }
}
impl TryFrom<AccountRow> for Account {
    type Error = RowConversionError;
    fn try_from(r: AccountRow) -> Result<Self, Self::Error> {
        let id = AccountId::from_uuid(uuid(&r.id, "accounts", &r.id)?);
        Ok(Self {
            id,
            budget_id: BudgetId::from_uuid(uuid(&r.budget_id, "accounts", &r.id)?),
            group_id: None,
            name: r.name,
            account_type: account_type(&r.account_type, &r.id)?,
            closed: boolean(r.closed, "accounts", &r.id)?,
            note: r.note,
            sort_order: r.sort_order,
            favorite: boolean(r.favorite, "accounts", &r.id)?,
        })
    }
}
impl ToSqlModel for Budget {
    type Model = BudgetRow;
    fn to_sql_model(&self) -> BudgetRow {
        BudgetRow {
            id: self.id.to_string(),
            name: self.name.clone(),
        }
    }
}
impl ToSqlModel for Account {
    type Model = AccountRow;
    fn to_sql_model(&self) -> AccountRow {
        AccountRow {
            id: self.id.to_string(),
            budget_id: self.budget_id.to_string(),
            name: self.name.clone(),
            account_type: match self.account_type {
                AccountType::Checking => "checking",
                AccountType::Savings => "savings",
                AccountType::Cash => "cash",
                AccountType::CreditCard => "credit_card",
                AccountType::Loan => "loan",
                AccountType::Asset => "asset",
                AccountType::Liability => "liability",
            }
            .into(),
            closed: i64::from(self.closed),
            note: self.note.clone(),
            sort_order: self.sort_order,
            favorite: i64::from(self.favorite),
        }
    }
}

pub fn validate_transaction(value: &Transaction) -> Result<(), RowConversionError> {
    value.validate().map_err(|_| {
        bad(
            "transactions",
            &value.id.to_string(),
            "invalid split aggregate",
        )
    })
}
pub fn validate_transfer_pair(
    left: &Transaction,
    right: &Transaction,
) -> Result<(), RowConversionError> {
    match (&left.body, &right.body) {
        (
            TransactionBody::Transfer {
                transfer_id: a,
                other_account_id: ao,
                other_amount: am,
                ..
            },
            TransactionBody::Transfer {
                transfer_id: b,
                other_account_id: bo,
                other_amount: bm,
                ..
            },
        ) if a == b
            && *ao == right.account_id
            && *bo == left.account_id
            && *am == right.amount
            && *bm == left.amount
            && left.amount.checked_neg().ok() == Some(right.amount) =>
        {
            Ok(())
        }
        _ => Err(bad(
            "transactions",
            &left.id.to_string(),
            "invalid transfer pair",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::model::ToSqlModel;
    use time::macros::date;

    #[test]
    fn primitive_and_enum_conversions_are_checked() {
        let id = Uuid::new_v4();
        assert_eq!(uuid(&id.to_string(), "x", "record").unwrap(), id);
        assert_eq!(
            date("2026-08-03", "x", "record").unwrap(),
            date!(2026 - 08 - 03)
        );
        assert_eq!(money(-123).minor_units(), -123);
        assert_eq!(
            account_type("credit_card", "record").unwrap(),
            AccountType::CreditCard
        );
        assert_eq!(
            clearance("reconciled", "record").unwrap(),
            Clearance::Reconciled
        );
        assert_eq!(approval("approved", "record").unwrap(), Approval::Approved);
        for error in [
            uuid("not an id", "transactions", "safe-id").unwrap_err(),
            date("bad", "transactions", "safe-id").unwrap_err(),
            account_type("bad", "safe-id").unwrap_err(),
        ] {
            let text = error.to_string();
            assert!(text.contains("safe-id"));
            assert!(!text.contains("memo") && !text.contains("payee") && !text.contains("$"));
        }
    }

    #[test]
    fn complete_account_round_trip() {
        let mut account = Account::new(BudgetId::new(), "Checking", AccountType::Checking);
        account.note = Some("private".into());
        account.closed = true;
        account.favorite = true;
        account.sort_order = 7;
        let restored = Account::try_from(account.to_sql_model()).unwrap();
        assert_eq!(restored, account);
    }

    fn transaction(account: AccountId, amount: i64, body: TransactionBody) -> Transaction {
        Transaction {
            id: TransactionId::new(),
            budget_id: BudgetId::new(),
            account_id: account,
            date: TransactionDate(date!(2026 - 08 - 03)),
            payee_id: None,
            amount: Money::from_minor_units(amount),
            memo: None,
            clearance: Clearance::Uncleared,
            approval: Approval::Approved,
            body,
            archived: false,
            voided: false,
        }
    }
    #[test]
    fn malformed_split_and_transfer_aggregates_are_rejected() {
        let category = CategoryId::new();
        let invalid = transaction(
            AccountId::new(),
            10,
            TransactionBody::Split {
                lines: vec![
                    Subtransaction {
                        category_id: category,
                        amount: Money::from_minor_units(9),
                        memo: None,
                    },
                    Subtransaction {
                        category_id: category,
                        amount: Money::ZERO,
                        memo: None,
                    },
                ],
            },
        );
        assert!(validate_transaction(&invalid).is_err());
        let left = transaction(
            AccountId::new(),
            -10,
            TransactionBody::OpeningBalance { category_id: None },
        );
        let right = transaction(
            AccountId::new(),
            10,
            TransactionBody::OpeningBalance { category_id: None },
        );
        assert!(validate_transfer_pair(&left, &right).is_err());
    }
}

#[cfg(test)]
mod investment_rejection_tests {
    use super::*;

    #[test]
    fn investment_deserialization_is_rejected() {
        assert!(account_type("investment", "account-id").is_err());
    }
}
