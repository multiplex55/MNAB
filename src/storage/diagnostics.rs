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

pub fn quick_check(connection: &Connection) -> Result<Vec<Finding>, rusqlite::Error> {
    pragma_text_check(connection, "quick_check")
}

pub fn integrity_check(connection: &Connection) -> Result<Vec<Finding>, rusqlite::Error> {
    pragma_text_check(connection, "integrity_check")
}

pub fn all(connection: &Connection, thorough: bool) -> Result<Vec<Finding>, rusqlite::Error> {
    let mut findings = if thorough {
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
        "SELECT m.account_id FROM credit_card_payment_categories m LEFT JOIN accounts a ON a.id=m.account_id LEFT JOIN categories c ON c.id=m.category_id WHERE a.id IS NULL OR c.id IS NULL OR a.account_type<>'credit_card'",
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
