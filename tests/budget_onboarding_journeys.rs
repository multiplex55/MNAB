//! End-to-end journeys through the fixed database and its authoritative projections.

use std::collections::BTreeSet;

use mnab::{
    app::{
        command::AppCommand, dispatcher::ActionCollector, lifecycle::DatabaseLifecycle,
        navigation::Workspace, portable_paths::PortablePaths, runtime::ApplicationRuntime,
        settings::SettingsSession, startup::StartupContext,
    },
    domain::BudgetMonth,
    storage::query_store::QueryStore,
};
use rusqlite::{Connection, params};
use uuid::Uuid;

fn dispatch(runtime: &mut ApplicationRuntime, command: AppCommand) {
    let mut actions = ActionCollector::default();
    actions.push(command);
    runtime.dispatch_collected(actions);
}

#[test]
fn first_run_wizard_commits_every_aggregate_and_opens_current_budget() {
    let root = tempfile::tempdir().unwrap();
    let paths = PortablePaths::from_executable(&root.path().join("mnab")).unwrap();
    assert!(
        !paths.database.exists(),
        "journey must begin without mnab.sqlite3"
    );
    let settings = SettingsSession::load(&paths.settings);
    let mut runtime = ApplicationRuntime::new(
        Some(paths.clone()),
        Some(settings),
        false,
        StartupContext {
            marker_was_absent: false,
            fixed_database_exists: false,
        },
    );
    assert_eq!(
        runtime.database_lifecycle(),
        DatabaseLifecycle::FirstRunRequired
    );

    let wizard = &mut runtime.view_mut().onboarding;
    wizard.step = 4;
    wizard.budget_name = "Household".into();
    wizard.account.name = "Checking".into();
    wizard.account.current_balance = "$5,000.00".into();
    wizard.account.balance_date = "2026-08-09".into();
    wizard.selected_categories = ["Rent", "Groceries", "Utilities"]
        .into_iter()
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    dispatch(&mut runtime, AppCommand::CompleteOnboarding);

    assert_eq!(runtime.database_lifecycle(), DatabaseLifecycle::Ready);
    assert!(
        runtime.worker_available(),
        "the committed session owns a running worker"
    );
    assert!(
        runtime.session().is_some(),
        "the initialized session was committed"
    );
    assert!(
        runtime.view().dialog.is_none(),
        "the wizard closes only after commit"
    );
    assert_eq!(runtime.view().navigation.workspace, Workspace::Budget);
    let current = time::OffsetDateTime::now_utc();
    assert_eq!(
        runtime.view().selected_month,
        BudgetMonth::new(current.year(), u8::from(current.month())).unwrap()
    );
    let db = Connection::open(&paths.database).unwrap();
    for (table, expected) in [
        ("budgets", 1),
        ("accounts", 1),
        ("transactions", 1),
        ("category_groups", 1),
        ("categories", 3),
    ] {
        let count: i64 = db
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, expected, "{table} persistence");
    }
    assert_eq!(
        db.query_row(
            "SELECT amount FROM transactions WHERE payee_snapshot='Opening Balance'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        500_000
    );
    let budget = runtime.session().unwrap().budget_id;
    let selected = runtime.view().selected_month;
    let projected = QueryStore::new(&db)
        .budget_month(
            budget,
            &format!("{:04}-{:02}", selected.year(), selected.month()),
        )
        .unwrap();
    assert_eq!(projected.ready_to_assign_cents, 500_000);
    drop(db);
    runtime.shutdown().unwrap();
}

#[test]
fn budget_assignment_spending_and_restart_reconstruct_identically() {
    let root = tempfile::tempdir().unwrap();
    let paths = PortablePaths::from_executable(&root.path().join("mnab")).unwrap();
    let initialized = mnab::service::onboarding_service::OnboardingService::initialize_database(
        &paths.database,
        mnab::service::onboarding_service::OnboardingRequest {
            budget_name: "Home".into(),
            account_name: "Checking".into(),
            account_type: mnab::domain::AccountType::Checking,
            opening_magnitude: mnab::domain::Money::from_minor_units(500_000),
            balance_date: mnab::domain::TransactionDate(time::macros::date!(2026 - 08 - 01)),
            group_name: "Cash Accounts".into(),
            note: None,
            categories: vec!["Rent".into(), "Groceries".into()],
        },
        || {},
    )
    .unwrap();
    let budget = initialized.session.budget_id;
    let mut worker = initialized.worker;
    worker.shutdown().unwrap();
    let db = Connection::open(&paths.database).unwrap();
    let category = |name: &str| -> String {
        db.query_row("SELECT id FROM categories WHERE name=?1", [name], |r| {
            r.get(0)
        })
        .unwrap()
    };
    let rent = category("Rent");
    let groceries = category("Groceries");
    for (category, cents) in [(&rent, 150_000), (&groceries, 50_000)] {
        db.execute("INSERT INTO budget_assignments(id,budget_id,category_id,budget_month,amount,created_at,modified_at) VALUES(?1,?2,?3,'2026-08',?4,'now','now')", params![Uuid::new_v4().to_string(), budget.to_string(), category, cents]).unwrap();
    }
    let checking: String = db
        .query_row("SELECT id FROM accounts", [], |r| r.get(0))
        .unwrap();
    db.execute("INSERT INTO transactions(id,budget_id,account_id,transaction_date,payee_snapshot,category_id,amount,cleared_state,approval_state,created_at,modified_at) VALUES(?1,?2,?3,'2026-08-09','Market',?4,-10000,'cleared','approved','now','now')", params![Uuid::new_v4().to_string(), budget.to_string(), checking, groceries]).unwrap();
    let month = BudgetMonth::new(2026, 8).unwrap();
    let first = QueryStore::new(&db)
        .budget_month(budget, "2026-08")
        .unwrap();
    assert_eq!(
        (
            first.assigned_cents,
            first.activity_cents,
            first.ready_to_assign_cents
        ),
        (200_000, -10_000, 300_000)
    );
    let grocery = first
        .rows
        .iter()
        .find(|row| row.name == "Groceries")
        .unwrap();
    assert_eq!(
        (
            grocery.assigned_cents,
            grocery.activity_cents,
            grocery.available_cents
        ),
        (50_000, -10_000, 40_000)
    );
    assert_eq!(first.month, month);
    drop(db);

    let reopened = Connection::open(&paths.database).unwrap();
    let reconstructed = QueryStore::new(&reopened)
        .budget_month(budget, "2026-08")
        .unwrap();
    assert_eq!(
        reconstructed, first,
        "restart must reconstruct, not rely on cached derived state"
    );
}
