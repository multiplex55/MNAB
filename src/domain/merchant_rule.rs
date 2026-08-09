use super::{AccountGroupId, AccountId, CategoryId, PayeeId, TransactionRuleId};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, Ord, PartialOrd)]
#[serde(rename_all = "snake_case")]
pub enum MerchantRuleOrigin {
    Learned,
    Explicit,
}

pub type TransactionRuleOrigin = MerchantRuleOrigin;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MerchantConfidence {
    Learning,
    High,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextMatch {
    Exact,
    Contains,
    Prefix,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransactionDirection {
    Inflow,
    Outflow,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RuleCondition {
    Merchant {
        value: String,
        match_type: TextMatch,
    },
    /// Read-only compatibility for persisted legacy rules. Authoring APIs must not create this.
    LegacyMerchantRegex {
        pattern: String,
    },
    Account(AccountId),
    AccountGroup(AccountGroupId),
    Direction(TransactionDirection),
    AmountExact(i64),
    AmountRange {
        minimum: i64,
        maximum: i64,
    },
    Memo {
        value: String,
        match_type: TextMatch,
    },
    ImportSource(String),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RuleAction {
    SetPayee {
        payee_id: PayeeId,
        display_name_snapshot: String,
    },
    SetCategory {
        category_id: CategoryId,
    },
    SetMemo {
        memo: String,
    },
    PrefixMemo {
        prefix: String,
    },
    SuffixMemo {
        suffix: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TransactionRule {
    pub id: TransactionRuleId,
    pub name: String,
    pub description: String,
    pub enabled: bool,
    pub priority: i32,
    pub origin: TransactionRuleOrigin,
    pub conditions: Vec<RuleCondition>,
    pub actions: Vec<RuleAction>,
    pub confidence: MerchantConfidence,
    /// Accepted learning observations, not speculative matches.
    pub usage_count: u32,
    pub match_count: u64,
    pub last_used_date: Option<time::Date>,
}

pub type MerchantRule = TransactionRule;

impl TransactionRule {
    #[must_use]
    pub fn account_scope(&self) -> Option<AccountId> {
        self.conditions.iter().find_map(|c| match c {
            RuleCondition::Account(id) => Some(*id),
            _ => None,
        })
    }
    #[must_use]
    pub fn normalized_merchant(&self) -> Option<&str> {
        self.conditions.iter().find_map(|c| match c {
            RuleCondition::Merchant { value, .. } => Some(value.as_str()),
            _ => None,
        })
    }
}

/// Deliberately conservative merchant identity.
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
    ((token.starts_with('#') || digits.len() >= 4)
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
