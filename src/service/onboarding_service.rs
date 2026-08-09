//! Atomic first-run database initialization.
use std::path::Path;

use crate::{
    app::session::{BudgetSession, SessionSummary},
    domain::*,
    storage::{diagnostics, migration, worker::StorageWorker},
};
use rusqlite::{Connection, params};

#[derive(Clone, Debug)]
pub struct OnboardingRequest {
    pub budget_name: String,
    pub account_name: String,
    pub account_type: AccountType,
    /// A non-negative user-entered amount. This layer applies the ledger sign once.
    pub opening_magnitude: Money,
    pub balance_date: TransactionDate,
    pub group_name: String,
    pub note: Option<String>,
    pub categories: Vec<String>,
}

pub struct InitializedBudget {
    pub session: BudgetSession,
    pub worker: StorageWorker,
}

#[derive(Debug, thiserror::Error)]
pub enum OnboardingError {
    #[error("budget and account names are required")]
    RequiredName,
    #[error("opening balance must be a non-negative magnitude")]
    NegativeMagnitude,
    #[error("first-run storage must be mnab-data/mnab.sqlite3")]
    InvalidDatabaseLocation,
    #[error("database already contains a budget")]
    AlreadyInitialized,
    #[error("storage setup failed: {0}")]
    Storage(String),
    #[error("initial database validation failed: {0}")]
    Validation(String),
}

pub struct OnboardingService;

impl OnboardingService {
    /// Initializes the fixed database atomically, then (and only then) starts its worker and
    /// returns a session ready for `ApplicationRuntime::commit_session`.
    pub fn initialize_database(
        database_path: &Path,
        request: OnboardingRequest,
        repaint: impl Fn() + Send + 'static,
    ) -> Result<InitializedBudget, OnboardingError> {
        validate_request(database_path, &request)?;
        if let Some(parent) = database_path.parent() {
            std::fs::create_dir_all(parent).map_err(storage)?;
        }
        let mut connection = Connection::open(database_path).map_err(storage)?;
        migration::migrate(&mut connection, database_path).map_err(storage)?;
        connection
            .execute_batch("PRAGMA foreign_keys=ON")
            .map_err(storage)?;
        let existing: i64 = connection
            .query_row("SELECT COUNT(*) FROM budgets", [], |row| row.get(0))
            .map_err(storage)?;
        if existing != 0 {
            return Err(OnboardingError::AlreadyInitialized);
        }

        let budget = Budget::new(request.budget_name.trim());
        let account = Account::new(budget.id, request.account_name.trim(), request.account_type);
        let transaction_id = TransactionId::new();
        let category_group_id = CategoryGroupId::new();
        let now = time::OffsetDateTime::now_utc().to_string();
        let opening = request
            .account_type
            .opening_amount(request.opening_magnitude)
            .map_err(|error| OnboardingError::Storage(error.to_string()))?;
        let tx = connection.transaction().map_err(storage)?;
        tx.execute(
            "INSERT INTO budgets(id,name,created_at,modified_at,archived) VALUES(?1,?2,?3,?3,0)",
            params![budget.id.to_string(), budget.name, now],
        )
        .map_err(storage)?;

        let defaults = [
            ("Cash Accounts", "cash"),
            ("Credit Accounts", "credit"),
            ("Loan Accounts", "loan"),
            ("Tracking Accounts", "asset"),
        ];
        let mut selected_group = None;
        for (position, (name, classification)) in defaults.into_iter().enumerate() {
            let id = AccountGroupId::new();
            if name == request.group_name {
                selected_group = Some(id);
            }
            tx.execute("INSERT INTO account_groups(id,budget_id,parent_group_id,name,classification,sort_order,collapsed) VALUES(?1,?2,NULL,?3,?4,?5,0)",
                params![id.to_string(), budget.id.to_string(), name, classification, position as i64]).map_err(storage)?;
        }
        // A custom selection remains supported without weakening the useful defaults.
        let group_id = if let Some(id) = selected_group {
            id
        } else {
            let id = AccountGroupId::new();
            tx.execute("INSERT INTO account_groups(id,budget_id,parent_group_id,name,classification,sort_order,collapsed) VALUES(?1,?2,NULL,?3,?4,4,0)",
                params![id.to_string(), budget.id.to_string(), request.group_name.trim(), classification(request.account_type)]).map_err(storage)?;
            id
        };
        tx.execute("INSERT INTO accounts(id,budget_id,group_id,name,account_type,sort_order,closed,note,favorite,created_at,modified_at) VALUES(?1,?2,?3,?4,?5,0,0,?6,0,?7,?7)",
            params![account.id.to_string(), budget.id.to_string(), group_id.to_string(), account.name, account_type(request.account_type), request.note.filter(|n| !n.trim().is_empty()), now]).map_err(storage)?;
        tx.execute("INSERT INTO transactions(id,budget_id,account_id,transaction_date,payee_snapshot,memo,amount,cleared_state,approval_state,created_at,modified_at,archived,voided) VALUES(?1,?2,?3,?4,'Opening Balance','Opening balance',?5,'cleared','approved',?6,?6,0,0)",
            params![transaction_id.to_string(), budget.id.to_string(), account.id.to_string(), request.balance_date.0.to_string(), opening.minor_units(), now]).map_err(storage)?;

        tx.execute("INSERT INTO category_groups(id,budget_id,name,sort_order,hidden) VALUES(?1,?2,'Starter Categories',0,0)",
            params![category_group_id.to_string(), budget.id.to_string()]).map_err(storage)?;
        for (position, name) in request
            .categories
            .iter()
            .map(|n| n.trim())
            .filter(|n| !n.is_empty())
            .enumerate()
        {
            tx.execute("INSERT INTO categories(id,budget_id,group_id,name,sort_order,hidden,archived) VALUES(?1,?2,?3,?4,?5,0,0)",
                params![CategoryId::new().to_string(), budget.id.to_string(), category_group_id.to_string(), name, position as i64]).map_err(storage)?;
        }
        let findings =
            diagnostics::run(&tx, diagnostics::DiagnosticSuite::Full).map_err(storage)?;
        if !findings.is_empty() {
            return Err(OnboardingError::Validation(
                findings
                    .into_iter()
                    .map(|f| f.summary)
                    .collect::<Vec<_>>()
                    .join("; "),
            ));
        }
        tx.commit().map_err(storage)?;
        drop(connection);

        let worker = StorageWorker::start(database_path, repaint).map_err(storage)?;
        Ok(InitializedBudget {
            session: BudgetSession {
                budget_id: budget.id,
                database_path: database_path.to_path_buf(),
                schema_version: migration::LATEST_SCHEMA_VERSION as u32,
                summary: SessionSummary {
                    budget_name: budget.name,
                    account_count: 1,
                },
            },
            worker,
        })
    }
}

fn validate_request(path: &Path, request: &OnboardingRequest) -> Result<(), OnboardingError> {
    if path.file_name().and_then(|x| x.to_str()) != Some("mnab.sqlite3")
        || path
            .parent()
            .and_then(Path::file_name)
            .and_then(|x| x.to_str())
            != Some("mnab-data")
    {
        return Err(OnboardingError::InvalidDatabaseLocation);
    }
    if request.budget_name.trim().is_empty() || request.account_name.trim().is_empty() {
        return Err(OnboardingError::RequiredName);
    }
    if request.opening_magnitude < Money::ZERO {
        return Err(OnboardingError::NegativeMagnitude);
    }
    Ok(())
}
fn storage(error: impl std::fmt::Display) -> OnboardingError {
    OnboardingError::Storage(error.to_string())
}
fn account_type(kind: AccountType) -> &'static str {
    match kind {
        AccountType::Checking => "checking",
        AccountType::Savings => "savings",
        AccountType::Cash => "cash",
        AccountType::CreditCard => "credit_card",
        AccountType::Loan => "loan",
        AccountType::Asset => "asset",
        AccountType::Liability => "liability",
    }
}
fn classification(kind: AccountType) -> &'static str {
    match kind {
        AccountType::Checking | AccountType::Savings | AccountType::Cash => "cash",
        AccountType::CreditCard => "credit",
        AccountType::Loan => "loan",
        AccountType::Asset => "asset",
        AccountType::Liability => "liability",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::OptionalExtension;
    use time::macros::date;
    fn request(kind: AccountType, cents: i64) -> OnboardingRequest {
        OnboardingRequest {
            budget_name: "Home".into(),
            account_name: "First".into(),
            account_type: kind,
            opening_magnitude: Money::from_minor_units(cents),
            balance_date: TransactionDate(date!(2026 - 08 - 06)),
            group_name: if matches!(kind, AccountType::CreditCard) {
                "Credit Accounts"
            } else {
                "Cash Accounts"
            }
            .into(),
            note: None,
            categories: vec!["Groceries".into()],
        }
    }
    #[test]
    fn cash_and_debt_are_atomic_and_activate_only_after_commit() {
        for (kind, expected) in [
            (AccountType::Checking, 12_500),
            (AccountType::CreditCard, -12_500),
        ] {
            let root = tempfile::tempdir().unwrap();
            let data = root.path().join("mnab-data");
            let path = data.join("mnab.sqlite3");
            let mut initialized =
                OnboardingService::initialize_database(&path, request(kind, 12_500), || {})
                    .unwrap();
            assert_eq!(initialized.session.database_path, path);
            initialized.worker.shutdown().unwrap();
            let connection = Connection::open(&path).unwrap();
            assert_eq!(
                connection
                    .query_row("SELECT amount FROM transactions", [], |r| r
                        .get::<_, i64>(0))
                    .unwrap(),
                expected
            );
            let month = crate::storage::query_store::QueryStore::new(&connection)
                .budget_month(initialized.session.budget_id, "2026-08")
                .unwrap();
            assert_eq!(
                month.ready_to_assign_cents,
                if kind == AccountType::Checking {
                    12_500
                } else {
                    0
                }
            );
            assert_eq!(
                connection
                    .query_row("PRAGMA foreign_key_check", [], |_| Ok(1))
                    .optional()
                    .unwrap(),
                None
            );
        }
    }
    #[test]
    fn validation_failures_do_not_insert_any_aggregate() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("mnab-data/mnab.sqlite3");
        let mut bad = request(AccountType::Checking, 100);
        bad.categories = vec!["same".into(), "same".into()];
        assert!(OnboardingService::initialize_database(&path, bad, || {}).is_err());
        let connection = Connection::open(path).unwrap();
        for table in [
            "budgets",
            "accounts",
            "transactions",
            "category_groups",
            "categories",
        ] {
            let count: i64 = connection
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
                .unwrap();
            assert_eq!(count, 0, "{table}");
        }
    }
    #[test]
    fn rejects_every_non_fixed_location_before_creating_it() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("other.sqlite3");
        assert!(matches!(
            OnboardingService::initialize_database(&path, request(AccountType::Checking, 0), || {}),
            Err(OnboardingError::InvalidDatabaseLocation)
        ));
        assert!(!path.exists());
    }
}
