use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum AmountColumns {
    Signed { amount: String },
    DebitCredit { debit: String, credit: String },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum DecimalStyle {
    Dot,
    Comma,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SignStyle {
    Leading,
    Trailing,
    Parentheses,
    Any,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum HeaderBehavior {
    Present,
    Absent,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum CsvEncoding {
    Utf8,
    Windows1252,
    Utf16Le,
    Utf16Be,
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
    #[serde(default)]
    pub source_identity: Option<String>,
    #[serde(default = "default_decimal")]
    pub decimal_style: DecimalStyle,
    #[serde(default = "default_sign")]
    pub sign_style: SignStyle,
    #[serde(default = "default_header")]
    pub header_behavior: HeaderBehavior,
    #[serde(default = "default_encoding")]
    pub encoding: CsvEncoding,
}
const fn default_decimal() -> DecimalStyle {
    DecimalStyle::Dot
}
const fn default_sign() -> SignStyle {
    SignStyle::Any
}
const fn default_header() -> HeaderBehavior {
    HeaderBehavior::Present
}
const fn default_encoding() -> CsvEncoding {
    CsvEncoding::Utf8
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
        let expected: BTreeSet<_> = signature_columns(&self.header_signature).collect();
        let found: BTreeSet<_> = signature_columns(&actual).collect();
        if !expected.is_empty() && expected.intersection(&found).count() * 2 >= expected.len() {
            PresetMatch::ConfirmationRequired
        } else {
            PresetMatch::Mismatch
        }
    }
}
fn signature_columns(signature: &str) -> impl Iterator<Item = &str> {
    signature
        .split_once(':')
        .and_then(|(_, rest)| rest.split_once(':'))
        .map_or("", |(_, columns)| columns)
        .split('|')
        .filter(|v| !v.is_empty())
}

#[derive(Clone, Debug, Default)]
pub struct CsvMappingStore {
    presets: BTreeMap<String, CsvMappingPreset>,
}
impl CsvMappingStore {
    #[must_use]
    pub fn suggested(&self, headers: &[String]) -> Option<&CsvMappingPreset> {
        let signature = header_signature(headers);
        self.presets.get(&signature)
    }
    pub fn save(&mut self, preset: CsvMappingPreset, replace: bool) -> Result<(), MappingError> {
        if self.presets.contains_key(&preset.header_signature) && !replace {
            return Err(MappingError::ReplacementRequired);
        }
        self.presets.insert(preset.header_signature.clone(), preset);
        Ok(())
    }
    #[must_use]
    pub fn all(&self) -> impl Iterator<Item = &CsvMappingPreset> {
        self.presets.values()
    }
}

#[must_use]
pub fn header_signature(headers: &[String]) -> String {
    // Order and duplicate columns are structural information.  Do not sort: two
    // exports with the same labels in a different layout are different sources.
    let normalized: Vec<_> = headers
        .iter()
        .map(|h| {
            h.trim()
                .to_lowercase()
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect();
    format!("v1:{}:{}", normalized.len(), normalized.join("|"))
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
    #[error("a mapping already exists for this source signature; explicit replacement is required")]
    ReplacementRequired,
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
            source_identity: None,
            decimal_style: DecimalStyle::Dot,
            sign_style: SignStyle::Any,
            header_behavior: HeaderBehavior::Present,
            encoding: CsvEncoding::Utf8,
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
    #[test]
    fn signature_preserves_structure_and_replacement_is_explicit() {
        let left = vec![" Date ".into(), "AMOUNT".into(), "Payee Name".into()];
        let reordered = vec!["amount".into(), "date".into(), "payee name".into()];
        assert_ne!(header_signature(&left), header_signature(&reordered));
        let mut store = CsvMappingStore::default();
        let mut preset = CsvMappingPreset {
            name: "Original".into(),
            header_signature: header_signature(&left),
            delimiter: b',',
            date_format: "[year]-[month]-[day]".into(),
            mapping: CsvMapping {
                date: "Date".into(),
                description: None,
                payee: None,
                memo: None,
                check_number: None,
                amount: AmountColumns::Signed {
                    amount: "AMOUNT".into(),
                },
            },
            source_identity: Some("bank-export".into()),
            decimal_style: DecimalStyle::Dot,
            sign_style: SignStyle::Any,
            header_behavior: HeaderBehavior::Present,
            encoding: CsvEncoding::Utf8,
        };
        store.save(preset.clone(), false).unwrap();
        preset.name = "Replacement".into();
        assert_eq!(
            store.save(preset.clone(), false),
            Err(MappingError::ReplacementRequired)
        );
        store.save(preset, true).unwrap();
        assert_eq!(store.suggested(&left).unwrap().name, "Replacement");
    }
}
