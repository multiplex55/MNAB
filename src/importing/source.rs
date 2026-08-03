use crate::domain::Money;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::Path};
use thiserror::Error;
use time::Date;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SourceLocation {
    CsvRow(u64),
    OfxTransaction { index: usize, fitid: Option<String> },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceAccount {
    pub bank_id: Option<String>,
    pub account_id: String,
    pub account_type: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ImportedTransaction {
    pub posted_date: Date,
    pub authorized_date: Option<Date>,
    pub amount: Money,
    pub payee: Option<String>,
    pub memo: Option<String>,
    pub fitid: Option<String>,
    pub check_number: Option<String>,
    pub transaction_type: Option<String>,
    pub source_account: Option<SourceAccount>,
    pub raw_fields: BTreeMap<String, String>,
    pub location: SourceLocation,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ImportedStatement {
    pub currency: Option<String>,
    pub account: Option<SourceAccount>,
    pub start_date: Option<Date>,
    pub end_date: Option<Date>,
    pub ledger_balance: Option<Money>,
    pub available_balance: Option<Money>,
    pub transactions: Vec<ImportedTransaction>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImportFormat {
    Ofx,
    Delimited,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Detection {
    Certain(ImportFormat),
    Ambiguous(Vec<ImportFormat>),
    Unknown,
}

/// Content is authoritative. The extension is only used to break a weak tie.
#[must_use]
pub fn detect(data: &[u8], name: Option<&Path>) -> Detection {
    let sample = String::from_utf8_lossy(&data[..data.len().min(8192)]);
    let trimmed = sample.trim_start_matches(['\u{feff}', '\0', ' ', '\t', '\r', '\n']);
    let upper = trimmed.to_ascii_uppercase();
    let ofx = upper.starts_with("OFXHEADER:")
        || upper.starts_with("<?XML")
        || upper.starts_with("<OFX")
        || upper.contains("\n<OFX>")
        || (upper.contains("<BANKMSGSRSV1>") && upper.contains("<STMTTRN>"));
    let delimited = plausible_delimited(trimmed);
    if ofx {
        return Detection::Certain(ImportFormat::Ofx);
    }
    if delimited {
        return Detection::Certain(ImportFormat::Delimited);
    }
    let hint = name
        .and_then(Path::extension)
        .and_then(|v| v.to_str())
        .map(str::to_ascii_lowercase);
    match hint.as_deref() {
        Some("qfx" | "qbo" | "ofx") => Detection::Ambiguous(vec![ImportFormat::Ofx]),
        Some("csv" | "tsv") => Detection::Ambiguous(vec![ImportFormat::Delimited]),
        _ => Detection::Unknown,
    }
}

fn plausible_delimited(text: &str) -> bool {
    let lines: Vec<_> = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .take(4)
        .collect();
    let Some(first) = lines.first() else {
        return false;
    };
    [',', '\t', ';'].into_iter().any(|d| {
        let n = first.matches(d).count();
        n > 0
            && lines
                .iter()
                .skip(1)
                .any(|line| line.matches(d).count() == n)
    })
}

#[derive(Debug, Error)]
pub enum ImportError {
    #[error("input exceeds the {limit} byte limit")]
    SizeLimit { limit: usize },
    #[error("could not decode input near byte {offset}: {message}")]
    Decode { offset: usize, message: String },
    #[error("{location}: invalid {field}: {message}")]
    Field {
        location: String,
        field: String,
        message: String,
    },
    #[error("input nesting exceeds the {limit} element limit")]
    DepthLimit { limit: usize },
    #[error("input exceeds the {limit} transaction limit")]
    TransactionLimit { limit: usize },
    #[error("unrecognized or structurally invalid input: {0}")]
    Structure(String),
}
