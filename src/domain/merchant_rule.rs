use super::{AccountId, CategoryId};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MerchantRuleOrigin {
    Explicit,
    Learned,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MerchantConfidence {
    Learning,
    High,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MerchantRule {
    pub normalized_merchant: String,
    pub account_id: Option<AccountId>,
    pub category_id: CategoryId,
    pub origin: MerchantRuleOrigin,
    pub confidence: MerchantConfidence,
    pub usage_count: u32,
    pub last_matched_date: Option<time::Date>,
    pub enabled: bool,
}

/// Deliberately conservative merchant identity. This is exact matching after
/// normalization, not fuzzy matching or a user-provided regular expression.
#[must_use]
pub fn normalize_merchant(value: &str) -> String {
    let mut words: Vec<String> = value
        .trim_matches(|c: char| c.is_ascii_punctuation() || c.is_whitespace())
        .split_whitespace()
        .map(|word| {
            word.trim_matches(|c: char| c.is_ascii_punctuation())
                .to_lowercase()
        })
        .filter(|word| !word.is_empty())
        .collect();
    while words.last().is_some_and(|word| is_transaction_suffix(word)) {
        words.pop();
    }
    words.join(" ")
}

fn is_transaction_suffix(word: &str) -> bool {
    let token = word.trim_matches(|c: char| c.is_ascii_punctuation());
    let digits = token.strip_prefix('#').unwrap_or(token);
    ((token.starts_with('#') || (digits.len() >= 4))
        && !digits.is_empty()
        && digits.chars().all(|c| c.is_ascii_digit()))
        || (["txn", "trans", "transaction", "ref"]
            .iter()
            .any(|p| digits.starts_with(p))
            && digits
                .chars()
                .skip_while(|c| c.is_ascii_alphabetic())
                .all(|c| c.is_ascii_digit())
            && digits.chars().any(|c| c.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn normalization_is_conservative() {
        assert_eq!(
            normalize_merchant("  --ACME   Market-- #12345 "),
            "acme market"
        );
        assert_eq!(normalize_merchant("Acme Markets"), "acme markets");
    }
}
