//! Read-only `SQLite` and budgeting-domain diagnostics.

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Warning,
    Error,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Finding {
    pub severity: Severity,
    pub check: String,
    pub table: Option<String>,
    pub entity_id: Option<String>,
    pub summary: String,
    /// Guidance never proposes deleting or rewriting financial records automatically.
    pub remediation: String,
}

/// Quick is suitable for a familiar, clean open. Full substitutes SQLite's
/// exhaustive integrity check and is required after an unclean shutdown or repair.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiagnosticSuite {
    Quick,
    Full,
}

pub fn quick_check(connection: &Connection) -> Result<Vec<Finding>, rusqlite::Error> {
    pragma_text_check(connection, "quick_check")
}

pub fn integrity_check(connection: &Connection) -> Result<Vec<Finding>, rusqlite::Error> {
    pragma_text_check(connection, "integrity_check")
}

pub fn all(connection: &Connection, thorough: bool) -> Result<Vec<Finding>, rusqlite::Error> {
    run(
        connection,
        if thorough {
            DiagnosticSuite::Full
        } else {
            DiagnosticSuite::Quick
        },
    )
}

/// Runs the named suite. Both suites include relational and financial checks;
/// only the underlying SQLite page-level check differs.
pub fn run(
    connection: &Connection,
    suite: DiagnosticSuite,
) -> Result<Vec<Finding>, rusqlite::Error> {
    let mut findings = if suite == DiagnosticSuite::Full {
        integrity_check(connection)?
    } else {
        quick_check(connection)?
    };
    foreign_keys(connection, &mut findings)?;
    query_entities(
        connection,
        &mut findings,
        "split_sum",
        "subtransactions",
        "SELECT t.id FROM transactions t JOIN subtransactions s ON s.transaction_id=t.id GROUP BY t.id HAVING SUM(s.amount)<>t.amount",
        "Split amounts do not equal the parent transaction amount.",
    )?;
    query_entities(
        connection,
        &mut findings,
        "transfer_pair",
        "transfer_links",
        "SELECT l.id FROM transfer_links l JOIN transactions a ON a.id=l.source_transaction_id JOIN transactions b ON b.id=l.destination_transaction_id WHERE a.amount<>-b.amount OR a.transfer_id<>l.id OR b.transfer_id<>l.id",
        "Transfer sides are not an equal-and-opposite linked pair.",
    )?;
    query_entities(
        connection,
        &mut findings,
        "managed_card_category",
        "credit_card_payment_categories",
        "SELECT m.account_id FROM credit_card_payment_categories m LEFT JOIN accounts a ON a.id=m.account_id LEFT JOIN categories c ON c.id=m.category_id WHERE a.id IS NULL OR c.id IS NULL OR a.account_type<>'credit_card' OR a.budget_id<>m.budget_id OR c.budget_id<>m.budget_id",
        "A managed payment category is missing or belongs to a non-card account.",
    )?;
    query_entities(
        connection,
        &mut findings,
        "duplicate_fitid",
        "transactions",
        "SELECT MIN(id) FROM transactions WHERE imported_fitid IS NOT NULL GROUP BY account_id,imported_fitid HAVING COUNT(*)>1",
        "More than one transaction has the same account/FITID identity.",
    )?;
    query_entities(
        connection,
        &mut findings,
        "reconciliation_association",
        "reconciliation_transactions",
        "SELECT rt.transaction_id FROM reconciliation_transactions rt JOIN transactions t ON t.id=rt.transaction_id JOIN reconciliations r ON r.id=rt.reconciliation_id WHERE t.account_id<>r.account_id OR t.reconciliation_id<>r.id",
        "A reconciliation association disagrees with its transaction or account.",
    )?;
    query_entities(
        connection,
        &mut findings,
        "invalid_target_recurrence",
        "targets",
        "SELECT id FROM targets WHERE (recurrence<>'none' AND target_type<>'upcoming_expense') OR recurrence NOT IN ('none','monthly','yearly')",
        "A target has a recurrence unsupported by its target kind.",
    )?;
    query_entities(
        connection,
        &mut findings,
        "invalid_schedule_interval",
        "scheduled_transactions",
        "SELECT id FROM scheduled_transactions WHERE (recurrence='custom_days' AND COALESCE(custom_interval_days,0)<=0) OR (recurrence<>'custom_days' AND custom_interval_days IS NOT NULL)",
        "A schedule has an invalid custom recurrence interval.",
    )?;
    query_entities(
        connection,
        &mut findings,
        "invalid_schedule_dates",
        "scheduled_transactions",
        "SELECT id FROM scheduled_transactions WHERE end_date IS NOT NULL AND end_date<start_date",
        "A schedule ends before it starts.",
    )?;
    query_entities(
        connection,
        &mut findings,
        "dangling_occurrence_disposition",
        "scheduled_occurrences",
        "SELECT o.id FROM scheduled_occurrences o LEFT JOIN transactions t ON t.id=o.transaction_id WHERE (o.disposition='entered' AND t.id IS NULL) OR (o.disposition<>'entered' AND o.transaction_id IS NOT NULL)",
        "A scheduled occurrence disposition does not agree with its entered transaction.",
    )?;
    query_entities(
        connection,
        &mut findings,
        "incomplete_import_identity",
        "import_identities",
        "SELECT id FROM import_identities WHERE normalized_fingerprint='' OR (fitid IS NULL AND (source_id IS NULL OR source_record_id IS NULL))",
        "An imported transaction is missing a usable source identity or fingerprint.",
    )?;
    query_entities(
        connection,
        &mut findings,
        "invalid_manual_match",
        "import_manual_matches",
        "SELECT m.candidate_id FROM import_manual_matches m LEFT JOIN staged_import_candidates c ON c.id=m.candidate_id LEFT JOIN import_batches b ON b.id=c.batch_id LEFT JOIN transactions t ON t.id=m.transaction_id LEFT JOIN import_decisions d ON d.candidate_id=m.candidate_id WHERE c.id IS NULL OR t.id IS NULL OR b.account_id<>t.account_id OR d.decision<>'manual_match' OR d.transaction_id<>m.transaction_id",
        "A manual import match is incomplete or crosses accounts.",
    )?;
    Ok(findings)
}

fn pragma_text_check(
    connection: &Connection,
    pragma: &str,
) -> Result<Vec<Finding>, rusqlite::Error> {
    let mut statement = connection.prepare(&format!("PRAGMA {pragma}"))?;
    let messages = statement
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(messages
        .into_iter()
        .filter(|message| message != "ok")
        .map(|message| Finding {
            severity: Severity::Error,
            check: pragma.to_owned(),
            table: None,
            entity_id: None,
            summary: message,
            remediation: "Stop editing this budget; preserve it and restore a validated backup."
                .into(),
        })
        .collect())
}

fn foreign_keys(connection: &Connection, findings: &mut Vec<Finding>) -> rusqlite::Result<()> {
    let mut statement = connection.prepare("PRAGMA foreign_key_check")?;
    let rows = statement.query_map([], |row| {
        Ok(Finding {
            severity: Severity::Error,
            check: "foreign_key_check".into(),
            table: Some(row.get(0)?),
            entity_id: Some(row.get::<_, i64>(1)?.to_string()),
            summary: format!("Referenced parent table {} is missing.", row.get::<_, String>(2)?),
            remediation: "Preserve the database and restore a validated backup; do not delete the orphan automatically.".into(),
        })
    })?;
    findings.extend(rows.collect::<Result<Vec<_>, _>>()?);
    Ok(())
}

fn query_entities(
    connection: &Connection,
    findings: &mut Vec<Finding>,
    check: &str,
    table: &str,
    sql: &str,
    summary: &str,
) -> rusqlite::Result<()> {
    let mut statement = connection.prepare(sql)?;
    let ids = statement
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    findings.extend(ids.into_iter().map(|id| Finding {
        severity: Severity::Error,
        check: check.into(),
        table: Some(table.into()),
        entity_id: Some(id),
        summary: summary.into(),
        remediation: "Review the identified record and restore a validated backup if its history cannot explain the inconsistency.".into(),
    }));
    Ok(())
}
