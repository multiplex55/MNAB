//! Persistent user journeys that deliberately exercise the fixed account-centric database.
//! These tests use only paths inside a temporary application-data directory, so a developer's
//! legacy `mnab-data/budgets` tree can never be observed or modified.

use std::path::Path;

use mnab::{
    app::{
        navigation::Workspace,
        settings::SettingsSession,
        startup::{StartupAccount, StartupDestination, resolve_destination},
    },
    domain::AccountId,
    storage::{migration::SCHEMA_FAMILY, open_primary},
};
use rusqlite::{Connection, params};
use tempfile::TempDir;
use uuid::Uuid;

const NOW: &str = "2026-08-08T12:00:00Z";
const TODAY: &str = "2026-08-08";

fn id() -> String {
    Uuid::new_v4().to_string()
}

struct Journey {
    data: TempDir,
    db_path: std::path::PathBuf,
    settings_path: std::path::PathBuf,
    db: Connection,
    budget: String,
    checking: String,
    starter_group: String,
    groceries: String,
}

impl Journey {
    fn first_launch() -> Self {
        let data = tempfile::tempdir().unwrap();
        let db_path = data.path().join("mnab.sqlite3");
        let settings_path = data.path().join("settings.json");
        assert!(
            !db_path.exists(),
            "first launch must start without a database"
        );
        assert!(!data.path().join("mnab-data/budgets").exists());
        let db = open_primary(&db_path).unwrap();
        let family: String = db
            .query_row(
                "SELECT value FROM application_metadata WHERE key='schema_family'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(family, SCHEMA_FAMILY);

        let budget = id();
        let checking = id();
        let cash_group = id();
        let starter_group = id();
        let groceries = id();
        let opening = id();
        let tx = db.unchecked_transaction().unwrap();
        tx.execute(
            "INSERT INTO budgets(id,name,created_at,modified_at) VALUES(?1,'Budget',?2,?2)",
            params![budget, NOW],
        )
        .unwrap();
        tx.execute("INSERT INTO account_groups(id,budget_id,name,classification,sort_order) VALUES(?1,?2,'Cash Accounts','cash',0)", params![cash_group, budget]).unwrap();
        tx.execute("INSERT INTO accounts(id,budget_id,group_id,name,account_type,sort_order,created_at,modified_at) VALUES(?1,?2,?3,'Checking','checking',0,?4,?4)", params![checking, budget, cash_group, NOW]).unwrap();
        tx.execute("INSERT INTO transactions(id,budget_id,account_id,transaction_date,memo,amount,cleared_state,approval_state,created_at,modified_at) VALUES(?1,?2,?3,?4,'Opening Balance',500000,'cleared','approved',?5,?5)", params![opening, budget, checking, TODAY, NOW]).unwrap();
        tx.execute("INSERT INTO category_groups(id,budget_id,name,sort_order) VALUES(?1,?2,'Starter Categories',0)", params![starter_group, budget]).unwrap();
        tx.execute("INSERT INTO categories(id,budget_id,group_id,name,sort_order) VALUES(?1,?2,?3,'Groceries',0)", params![groceries, budget, starter_group]).unwrap();
        tx.commit().unwrap();

        let checking_id: AccountId = checking.parse().unwrap();
        assert_eq!(
            resolve_destination(
                Some(&checking),
                &[StartupAccount {
                    id: checking_id,
                    favorite: false,
                    closed: false
                }]
            ),
            StartupDestination::Workspace(Workspace::Account(checking_id)),
            "finishing onboarding opens the Checking register"
        );
        Self {
            data,
            db_path,
            settings_path,
            db,
            budget,
            checking,
            starter_group,
            groceries,
        }
    }

    fn balance(&self, account: &str) -> i64 {
        self.db.query_row(
            "SELECT COALESCE(SUM(amount),0) FROM transactions WHERE account_id=?1 AND archived=0 AND voided=0",
            [account], |row| row.get(0),
        ).unwrap()
    }

    fn category_effect(&self, account: &str, category: &str) -> i64 {
        self.db.query_row(
            "SELECT COALESCE(SUM(amount),0) FROM transactions WHERE account_id=?1 AND category_id=?2",
            params![account, category], |row| row.get(0),
        ).unwrap()
    }

    fn add_account(&self, name: &str, kind: &str, group: &str, order: i64) -> String {
        let account = id();
        self.db.execute("INSERT INTO accounts(id,budget_id,group_id,name,account_type,sort_order,created_at,modified_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?7)", params![account, self.budget, group, name, kind, order, NOW]).unwrap();
        account
    }

    fn transaction(
        &self,
        account: &str,
        amount: i64,
        payee: &str,
        category: Option<&str>,
        approval: &str,
    ) -> String {
        let transaction = id();
        self.db.execute("INSERT INTO transactions(id,budget_id,account_id,transaction_date,payee_snapshot,category_id,amount,cleared_state,approval_state,created_at,modified_at) VALUES(?1,?2,?3,?4,?5,?6,?7,'cleared',?8,?9,?9)", params![transaction, self.budget, account, TODAY, payee, category, amount, approval, NOW]).unwrap();
        transaction
    }

    fn transfer(
        &self,
        source: &str,
        destination: &str,
        amount: i64,
        category: Option<&str>,
    ) -> (String, String, String) {
        assert!(amount > 0);
        let transfer = id();
        let source_leg = self.transaction(source, -amount, "Transfer", category, "approved");
        let destination_leg =
            self.transaction(destination, amount, "Transfer", category, "approved");
        self.db
            .execute(
                "UPDATE transactions SET transfer_id=?1 WHERE id IN (?2,?3)",
                params![transfer, source_leg, destination_leg],
            )
            .unwrap();
        self.db.execute("INSERT INTO transfers(id,budget_id,source_transaction_id,destination_transaction_id) VALUES(?1,?2,?3,?4)", params![transfer, self.budget, source_leg, destination_leg]).unwrap();
        (transfer, source_leg, destination_leg)
    }
}

#[test]
fn account_centric_first_launch_through_restart_user_journeys() {
    let mut app = Journey::first_launch();
    assert_eq!(app.balance(&app.checking), 500_000);

    // Nested groups, ordering, collapsed state, and selection survive a real close/reopen cycle.
    let daily = id();
    let cash = id();
    app.db.execute("INSERT INTO account_groups(id,budget_id,name,classification,sort_order,collapsed) VALUES(?1,?2,'Daily Finances','cash',10,1)", params![daily, app.budget]).unwrap();
    app.db.execute("INSERT INTO account_groups(id,budget_id,parent_group_id,name,classification,sort_order) VALUES(?1,?2,?3,'Cash Accounts (Daily)','cash',20)", params![cash, app.budget, daily]).unwrap();
    app.db
        .execute(
            "UPDATE accounts SET group_id=?1,sort_order=10 WHERE id=?2",
            params![cash, app.checking],
        )
        .unwrap();
    let savings = app.add_account("Savings", "savings", &cash, 20);
    {
        let mut settings = SettingsSession::load(&app.settings_path);
        settings.value_mut().last_selected_account_id = Some(savings.clone());
        settings.value_mut().last_workspace = Some("account".into());
        settings.value_mut().collapsed_account_groups = vec![daily.clone()];
        settings.save().unwrap();
    }
    drop(app.db);
    app.db = open_primary(&app.db_path).unwrap();
    let restored = SettingsSession::load(&app.settings_path);
    assert_eq!(
        restored.value().last_selected_account_id.as_deref(),
        Some(savings.as_str())
    );
    assert_eq!(restored.value().collapsed_account_groups, [daily.clone()]);
    let group_state: (Option<String>, i64, bool) = app
        .db
        .query_row(
            "SELECT parent_group_id,sort_order,collapsed FROM account_groups WHERE id=?1",
            [&cash],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(group_state, (Some(daily), 20, false));

    // A Savings-scoped goal receives the categorized transfer and is reduced by spending.
    let vacation = id();
    app.db.execute("INSERT INTO categories(id,budget_id,group_id,name,sort_order) VALUES(?1,?2,?3,'Vacation',10)", params![vacation, app.budget, app.starter_group]).unwrap();
    app.db.execute("INSERT INTO category_goals(id,budget_id,category_id,account_id,goal_type,amount,created_at,modified_at) VALUES(?1,?2,?3,?4,'balance',300000,?5,?5)", params![id(), app.budget, vacation, savings, NOW]).unwrap();
    app.transfer(&app.checking, &savings, 50_000, Some(&vacation));
    assert_eq!(app.balance(&savings), 50_000);
    assert_eq!(app.balance(&app.checking), 450_000);
    assert_eq!(app.category_effect(&savings, &vacation), 50_000);
    app.transaction(&savings, -20_000, "Hotel", Some(&vacation), "approved");
    assert_eq!(app.balance(&savings), 30_000);
    assert_eq!(app.category_effect(&savings, &vacation), 30_000);

    // High-confidence categorization does not imply approval; approval records rule usage.
    let rule = id();
    app.db.execute("INSERT INTO merchant_rules(id,budget_id,account_id,pattern,match_type,category_id,origin,confidence,usage_count,created_at,modified_at) VALUES(?1,?2,?3,'kroger','exact',?4,'learned','high',3,?5,?5)", params![rule, app.budget, app.checking, app.groceries, NOW]).unwrap();
    let imported = app.transaction(
        &app.checking,
        -10_000,
        "Kroger",
        Some(&app.groceries),
        "unapproved",
    );
    let imported_state: (String, String) = app
        .db
        .query_row(
            "SELECT category_id,approval_state FROM transactions WHERE id=?1",
            [&imported],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(imported_state, (app.groceries.clone(), "unapproved".into()));
    app.db
        .execute(
            "UPDATE transactions SET approval_state='approved' WHERE id=?1",
            [&imported],
        )
        .unwrap();
    app.db.execute("UPDATE merchant_rules SET usage_count=usage_count+1,last_matched_date=?1,modified_at=?2 WHERE id=?3", params![TODAY, NOW, rule]).unwrap();
    assert_eq!(
        app.db
            .query_row(
                "SELECT usage_count FROM merchant_rules WHERE id=?1",
                [&rule],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        4
    );

    // Credit-card debt and payment legs use one durable transfer identity.
    let credit = id();
    app.db.execute("INSERT INTO account_groups(id,budget_id,name,classification,sort_order) VALUES(?1,?2,'Credit Cards','credit',30)", params![credit, app.budget]).unwrap();
    let visa = app.add_account("Visa", "credit_card", &credit, 0);
    let electronics = id();
    app.db.execute("INSERT INTO categories(id,budget_id,group_id,name,sort_order) VALUES(?1,?2,?3,'Electronics',20)", params![electronics, app.budget, app.starter_group]).unwrap();
    app.transaction(&visa, -90_000, "Best Buy", Some(&electronics), "approved");
    assert_eq!(app.balance(&visa), -90_000);
    let (payment, source_leg, destination_leg) = app.transfer(&app.checking, &visa, 90_000, None);
    assert_eq!(app.balance(&app.checking), 350_000);
    assert_eq!(app.balance(&visa), 0);
    let linked: (String, String) = app
        .db
        .query_row(
            "SELECT source_transaction_id,destination_transaction_id FROM transfers WHERE id=?1",
            [&payment],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(linked, (source_leg, destination_leg));

    // Complete reconciliation retains both its immutable identity and included-row history.
    let reconciliation = id();
    app.db.execute("INSERT INTO reconciliations(id,budget_id,account_id,statement_date,ending_balance,created_at,calculated_cleared_balance,difference,state,completed_at) VALUES(?1,?2,?3,?4,350000,?5,350000,0,'completed',?5)", params![reconciliation, app.budget, app.checking, TODAY, NOW]).unwrap();
    let cleared_ids: Vec<String> = {
        let mut statement = app
            .db
            .prepare("SELECT id FROM transactions WHERE account_id=?1 AND cleared_state='cleared'")
            .unwrap();
        statement
            .query_map([&app.checking], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    };
    for transaction in &cleared_ids {
        app.db.execute("INSERT INTO reconciliation_transactions(reconciliation_id,budget_id,transaction_id,included_at) VALUES(?1,?2,?3,?4)", params![reconciliation, app.budget, transaction, NOW]).unwrap();
        app.db.execute("UPDATE transactions SET cleared_state='reconciled',reconciliation_id=?1 WHERE id=?2", params![reconciliation, transaction]).unwrap();
    }
    let history_count: i64 = app
        .db
        .query_row(
            "SELECT COUNT(*) FROM reconciliation_transactions WHERE reconciliation_id=?1",
            [&reconciliation],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(history_count as usize, cleared_ids.len());
    assert!(history_count > 0);

    // Closing with Savings selected restores the fixed Budget and its account register.
    drop(app.db);
    app.db = open_primary(&app.db_path).unwrap();
    let settings = SettingsSession::load(&app.settings_path);
    let budget_name: String = app
        .db
        .query_row("SELECT name FROM budgets WHERE id=?1", [&app.budget], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(budget_name, "Budget");
    let savings_id: AccountId = savings.parse().unwrap();
    assert_eq!(
        resolve_destination(
            settings.value().last_selected_account_id.as_deref(),
            &[StartupAccount {
                id: savings_id,
                favorite: false,
                closed: false
            }]
        ),
        StartupDestination::Workspace(Workspace::Account(savings_id))
    );
    assert_eq!(app.category_effect(&savings, &vacation), 30_000);
    assert_isolated(app.data.path());
}

fn assert_isolated(data_path: &Path) {
    assert_eq!(
        data_path.join("mnab.sqlite3").file_name().unwrap(),
        "mnab.sqlite3"
    );
    assert!(!data_path.join("mnab-data").exists());
    assert!(!data_path.join("budgets").exists());
}
