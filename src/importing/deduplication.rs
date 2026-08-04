use super::source::ImportedTransaction;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use time::Date;

pub const FINGERPRINT_VERSION: u8 = 1;
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum CandidateClassification {
    New,
    ExactImportIdDuplicate,
    StrongDateAmountPayeeMatch,
    PossibleManualMatch,
    AccountMismatch,
    AmbiguousMatch,
    Invalid,
    Ignored,
}
impl CandidateClassification {
    #[allow(non_upper_case_globals)]
    pub const ExactDuplicate: Self = Self::ExactImportIdDuplicate;
    #[allow(non_upper_case_globals)]
    pub const PossibleDuplicate: Self = Self::StrongDateAmountPayeeMatch;
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MatchEvidence {
    pub kind: String,
    pub detail: String,
    pub weight: u8,
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
    pub evidence: Vec<MatchEvidence>,
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
    let mut suggestions: Vec<(usize, CandidateClassification, Vec<MatchEvidence>)> = Vec::new();
    if let Some(fitid) = candidate.fitid.as_deref() {
        for (i, e) in existing
            .iter()
            .enumerate()
            .filter(|(_, e)| e.fitid.as_deref() == Some(fitid))
        {
            let class = if e.account_key == account_key {
                CandidateClassification::ExactImportIdDuplicate
            } else {
                CandidateClassification::AccountMismatch
            };
            suggestions.push((
                i,
                class,
                vec![
                    evidence("import_id", format!("FITID {fitid}"), 100),
                    evidence(
                        "account",
                        account_evidence(account_key, &e.account_key),
                        if class == CandidateClassification::AccountMismatch {
                            95
                        } else {
                            100
                        },
                    ),
                ],
            ));
        }
    }
    if let Some(id) = source_identifier {
        for (i, e) in existing
            .iter()
            .enumerate()
            .filter(|(_, e)| e.source_identifier.as_deref() == Some(id))
        {
            let class = if e.account_key == account_key {
                CandidateClassification::ExactImportIdDuplicate
            } else {
                CandidateClassification::AccountMismatch
            };
            suggestions.push((
                i,
                class,
                vec![
                    evidence("source_record_id", id.to_owned(), 100),
                    evidence(
                        "account",
                        account_evidence(account_key, &e.account_key),
                        if class == CandidateClassification::AccountMismatch {
                            95
                        } else {
                            100
                        },
                    ),
                ],
            ));
        }
    }
    let fp = fingerprint(account_key, candidate);
    for (i, _e) in existing
        .iter()
        .enumerate()
        .filter(|(_, e)| e.fingerprint == fp)
    {
        suggestions.push((
            i,
            CandidateClassification::ExactImportIdDuplicate,
            vec![evidence(
                "fingerprint",
                "same normalized date/amount/payee/account".into(),
                90,
            )],
        ));
    }
    let payee = normalize_payee(candidate.payee.as_deref().unwrap_or(""));
    for (i, e) in existing.iter().enumerate() {
        let same_amount = e.amount_minor == candidate.amount.minor_units();
        let date_delta = (e.date.to_julian_day() - candidate.posted_date.to_julian_day()).abs();
        let payee_similarity = similarity(&payee, &normalize_payee(&e.payee));
        if same_amount && date_delta <= 1 && payee_similarity >= 0.92 {
            suggestions.push((
                i,
                if e.account_key == account_key {
                    CandidateClassification::StrongDateAmountPayeeMatch
                } else {
                    CandidateClassification::AccountMismatch
                },
                vec![
                    evidence("amount", "same amount".into(), 80),
                    evidence("date", format!("{date_delta} day(s) apart"), 70),
                    evidence("payee", format!("similarity {payee_similarity:.2}"), 60),
                ],
            ));
        } else if same_amount && date_delta <= 3 && payee_similarity >= 0.80 {
            suggestions.push((
                i,
                if e.account_key == account_key {
                    CandidateClassification::PossibleManualMatch
                } else {
                    CandidateClassification::AccountMismatch
                },
                vec![
                    evidence("amount", "same amount".into(), 60),
                    evidence("date", format!("{date_delta} day(s) apart"), 50),
                    evidence("payee", format!("similarity {payee_similarity:.2}"), 40),
                ],
            ));
        }
    }
    suggestions.sort_by(|a, b| {
        rank(a.1)
            .cmp(&rank(b.1))
            .then(a.0.cmp(&b.0))
            .then(evidence_key(&a.2).cmp(&evidence_key(&b.2)))
    });
    if suggestions.len() > 1 && rank(suggestions[0].1) == rank(suggestions[1].1) {
        let mut ev = suggestions[0].2.clone();
        ev.push(evidence(
            "ambiguity",
            "multiple equally strong matches".into(),
            99,
        ));
        return build(
            CandidateClassification::AmbiguousMatch,
            Some(suggestions[0].0),
            ev,
        );
    }
    suggestions.into_iter().next().map_or(
        DeduplicationResult {
            classification: CandidateClassification::New,
            matched_index: None,
            reason: None,
            evidence: Vec::new(),
        },
        |(i, class, evidence)| build(class, Some(i), evidence),
    )
}
fn rank(c: CandidateClassification) -> u8 {
    match c {
        CandidateClassification::ExactImportIdDuplicate => 0,
        CandidateClassification::AccountMismatch => 1,
        CandidateClassification::StrongDateAmountPayeeMatch => 2,
        CandidateClassification::PossibleManualMatch => 3,
        CandidateClassification::AmbiguousMatch => 4,
        CandidateClassification::New => 5,
        CandidateClassification::Invalid => 6,
        CandidateClassification::Ignored => 7,
    }
}
fn evidence(kind: &str, detail: String, weight: u8) -> MatchEvidence {
    MatchEvidence {
        kind: kind.into(),
        detail,
        weight,
    }
}
fn account_evidence(want: &str, got: &str) -> String {
    if want == got {
        format!("same account {want}")
    } else {
        format!("candidate account {want}, existing account {got}")
    }
}
fn evidence_key(e: &[MatchEvidence]) -> String {
    e.iter()
        .map(|v| format!("{}:{}:{}", v.weight, v.kind, v.detail))
        .collect::<Vec<_>>()
        .join("|")
}
fn build(
    classification: CandidateClassification,
    matched_index: Option<usize>,
    mut evidence: Vec<MatchEvidence>,
) -> DeduplicationResult {
    evidence.sort_by(|a, b| {
        b.weight
            .cmp(&a.weight)
            .then(a.kind.cmp(&b.kind))
            .then(a.detail.cmp(&b.detail))
    });
    let reason = Some(
        evidence
            .iter()
            .map(|e| format!("{}: {}", e.kind, e.detail))
            .collect::<Vec<_>>()
            .join("; "),
    );
    DeduplicationResult {
        classification,
        matched_index,
        reason,
        evidence,
    }
}

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
