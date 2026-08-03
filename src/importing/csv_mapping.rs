use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum AmountColumns {
    Signed { amount: String },
    DebitCredit { debit: String, credit: String },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CsvMapping {
    pub date: String,
    pub description: Option<String>,
    pub payee: Option<String>,
    pub memo: Option<String>,
    pub check_number: Option<String>,
    pub amount: AmountColumns,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CsvMappingPreset {
    pub name: String,
    pub header_signature: String,
    pub delimiter: u8,
    pub date_format: String,
    pub mapping: CsvMapping,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PresetMatch {
    Exact,
    ConfirmationRequired,
    Mismatch,
}

impl CsvMappingPreset {
    #[must_use]
    pub fn matches_headers(&self, headers: &[String]) -> PresetMatch {
        let actual = header_signature(headers);
        if actual == self.header_signature {
            return PresetMatch::Exact;
        }
        let expected: BTreeSet<_> = self.header_signature.split('|').collect();
        let found: BTreeSet<_> = actual.split('|').collect();
        if !expected.is_empty() && expected.intersection(&found).count() * 2 >= expected.len() {
            PresetMatch::ConfirmationRequired
        } else {
            PresetMatch::Mismatch
        }
    }
}

#[must_use]
pub fn header_signature(headers: &[String]) -> String {
    let mut normalized: Vec<_> = headers
        .iter()
        .map(|h| {
            h.trim()
                .to_lowercase()
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect();
    normalized.sort();
    normalized.join("|")
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum MappingError {
    #[error("required date column is missing")]
    MissingDate,
    #[error("amount mapping is incomplete")]
    MissingAmount,
    #[error("column '{0}' is assigned more than once")]
    Duplicate(String),
    #[error("mapped column '{0}' does not exist")]
    Unknown(String),
}

pub fn validate(mapping: &CsvMapping, headers: &[String]) -> Result<(), MappingError> {
    if mapping.date.trim().is_empty() {
        return Err(MappingError::MissingDate);
    }
    let mut assigned = BTreeMap::<String, usize>::new();
    let mut add = |column: &str| {
        *assigned.entry(column.to_owned()).or_default() += 1;
    };
    add(&mapping.date);
    for c in [
        &mapping.description,
        &mapping.payee,
        &mapping.memo,
        &mapping.check_number,
    ]
    .into_iter()
    .flatten()
    {
        add(c);
    }
    match &mapping.amount {
        AmountColumns::Signed { amount } => {
            if amount.is_empty() {
                return Err(MappingError::MissingAmount);
            }
            add(amount);
        }
        AmountColumns::DebitCredit { debit, credit } => {
            if debit.is_empty() || credit.is_empty() {
                return Err(MappingError::MissingAmount);
            }
            add(debit);
            add(credit);
        }
    }
    if let Some((name, _)) = assigned.iter().find(|(_, count)| **count > 1) {
        return Err(MappingError::Duplicate(name.clone()));
    }
    if let Some(name) = assigned
        .keys()
        .find(|name| !headers.iter().any(|h| h.as_str() == name.as_str()))
    {
        return Err(MappingError::Unknown(name.clone()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preset_round_trip_and_partial_requires_confirmation() {
        let headers = vec!["Date".into(), "Amount".into(), "Payee".into()];
        let preset = CsvMappingPreset {
            name: "Bank".into(),
            header_signature: header_signature(&headers),
            delimiter: b',',
            date_format: "[year]-[month]-[day]".into(),
            mapping: CsvMapping {
                date: "Date".into(),
                description: None,
                payee: Some("Payee".into()),
                memo: None,
                check_number: None,
                amount: AmountColumns::Signed {
                    amount: "Amount".into(),
                },
            },
        };
        let json = serde_json::to_string(&preset).unwrap();
        assert_eq!(
            serde_json::from_str::<CsvMappingPreset>(&json).unwrap(),
            preset
        );
        assert_eq!(preset.matches_headers(&headers), PresetMatch::Exact);
        assert_eq!(
            preset.matches_headers(&headers[..2]),
            PresetMatch::ConfirmationRequired
        );
    }
}
