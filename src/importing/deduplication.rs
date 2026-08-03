use super::source::ImportedTransaction;
use sha2::{Digest, Sha256};
use time::Date;

pub const FINGERPRINT_VERSION: u8 = 1;
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CandidateClassification {
    New,
    PossibleDuplicate,
    ExactDuplicate,
    PossibleManualMatch,
    Invalid,
    Ignored,
}

#[derive(Clone, Debug)]
pub struct ExistingImport {
    pub account_key: String,
    pub fitid: Option<String>,
    pub source_identifier: Option<String>,
    pub fingerprint: String,
    pub date: Date,
    pub amount_minor: i64,
    pub payee: String,
}

#[derive(Clone, Debug)]
pub struct DeduplicationResult {
    pub classification: CandidateClassification,
    pub matched_index: Option<usize>,
    pub reason: Option<String>,
}

#[must_use]
pub fn normalize_payee(value: &str) -> String {
    value
        .chars()
        .flat_map(char::to_lowercase)
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .filter(|token| !matches!(*token, "the" | "inc" | "llc" | "ltd"))
        .collect::<Vec<_>>()
        .join(" ")
}

#[must_use]
pub fn fingerprint(account_key: &str, transaction: &ImportedTransaction) -> String {
    let input = format!(
        "v{FINGERPRINT_VERSION}\0{}\0{}\0{}\0{}",
        account_key.trim().to_lowercase(),
        transaction.posted_date,
        transaction.amount.minor_units(),
        normalize_payee(transaction.payee.as_deref().unwrap_or(""))
    );
    format!(
        "v{FINGERPRINT_VERSION}:{}",
        hex(&Sha256::digest(input.as_bytes()))
    )
}

#[must_use]
pub fn classify(
    account_key: &str,
    source_identifier: Option<&str>,
    candidate: &ImportedTransaction,
    existing: &[ExistingImport],
) -> DeduplicationResult {
    if let Some(fitid) = candidate.fitid.as_deref() {
        if let Some((i, _)) = existing
            .iter()
            .enumerate()
            .find(|(_, e)| e.account_key == account_key && e.fitid.as_deref() == Some(fitid))
        {
            return result(
                CandidateClassification::ExactDuplicate,
                i,
                "same account and FITID",
            );
        }
    }
    if let Some(id) = source_identifier {
        if let Some((i, _)) = existing.iter().enumerate().find(|(_, e)| {
            e.account_key == account_key && e.source_identifier.as_deref() == Some(id)
        }) {
            return result(
                CandidateClassification::ExactDuplicate,
                i,
                "same source identifier",
            );
        }
    }
    let fp = fingerprint(account_key, candidate);
    if let Some((i, _)) = existing
        .iter()
        .enumerate()
        .find(|(_, e)| e.fingerprint == fp)
    {
        return result(
            CandidateClassification::ExactDuplicate,
            i,
            "exact normalized fingerprint",
        );
    }
    let payee = normalize_payee(candidate.payee.as_deref().unwrap_or(""));
    if let Some((i, _)) = existing.iter().enumerate().find(|(_, e)| {
        e.account_key == account_key
            && e.amount_minor == candidate.amount.minor_units()
            && (e.date.to_julian_day() - candidate.posted_date.to_julian_day()).abs() <= 3
            && similarity(&payee, &normalize_payee(&e.payee)) >= 0.80
    }) {
        return result(
            CandidateClassification::PossibleManualMatch,
            i,
            "same amount, date within 3 days, and similar payee",
        );
    }
    DeduplicationResult {
        classification: CandidateClassification::New,
        matched_index: None,
        reason: None,
    }
}
fn result(classification: CandidateClassification, i: usize, reason: &str) -> DeduplicationResult {
    DeduplicationResult {
        classification,
        matched_index: Some(i),
        reason: Some(reason.into()),
    }
}
/// Deterministic normalized Levenshtein similarity in the range 0..=1.
#[must_use]
pub fn similarity(a: &str, b: &str) -> f64 {
    let max = a.chars().count().max(b.chars().count());
    if max == 0 {
        return 1.0;
    }
    1.0 - levenshtein(a, b) as f64 / max as f64
}
fn levenshtein(a: &str, b: &str) -> usize {
    let b: Vec<_> = b.chars().collect();
    let mut row: Vec<_> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut previous = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let old = row[j + 1];
            row[j + 1] = (row[j + 1] + 1)
                .min(row[j] + 1)
                .min(previous + usize::from(ca != *cb));
            previous = old;
        }
    }
    row[b.len()]
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
