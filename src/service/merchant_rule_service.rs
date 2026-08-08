use crate::domain::*;
use std::collections::HashMap;

#[derive(Clone, Debug, Default)]
pub struct MerchantRuleBook {
    pub rules: Vec<MerchantRule>,
}

impl MerchantRuleBook {
    #[must_use]
    pub fn match_high_confidence(
        &self,
        merchant: &str,
        account: AccountId,
    ) -> Option<&MerchantRule> {
        let identity = normalize_merchant(merchant);
        self.rules
            .iter()
            .filter(|r| {
                r.enabled
                    && r.normalized_merchant == identity
                    && r.account_id.is_none_or(|scope| scope == account)
                    && r.confidence == MerchantConfidence::High
            })
            .max_by_key(|r| {
                (
                    r.origin == MerchantRuleOrigin::Explicit,
                    r.account_id.is_some(),
                    r.usage_count,
                )
            })
    }

    /// Records an approved outcome. Explicit rules are never silently changed.
    /// A mature learned rule only changes after three consistent corrections.
    pub fn record_approved(
        &mut self,
        merchant: &str,
        account: AccountId,
        category: CategoryId,
        date: time::Date,
    ) {
        let normalized = normalize_merchant(merchant);
        if let Some(rule) = self.rules.iter_mut().find(|r| {
            r.enabled
                && r.origin == MerchantRuleOrigin::Learned
                && r.normalized_merchant == normalized
                && r.account_id == Some(account)
                && r.category_id == category
        }) {
            rule.usage_count = rule.usage_count.saturating_add(1);
            rule.last_matched_date = Some(date);
            if rule.usage_count >= 3 {
                rule.confidence = MerchantConfidence::High;
            }
            return;
        }
        self.rules.push(MerchantRule {
            normalized_merchant: normalized,
            account_id: Some(account),
            category_id: category,
            origin: MerchantRuleOrigin::Learned,
            confidence: MerchantConfidence::Learning,
            usage_count: 1,
            last_matched_date: Some(date),
            enabled: true,
        });
    }

    #[must_use]
    pub fn learned_candidates(&self) -> HashMap<(&str, Option<AccountId>), Vec<&MerchantRule>> {
        let mut result = HashMap::new();
        for rule in self
            .rules
            .iter()
            .filter(|r| r.origin == MerchantRuleOrigin::Learned)
        {
            result
                .entry((rule.normalized_merchant.as_str(), rule.account_id))
                .or_insert_with(Vec::new)
                .push(rule);
        }
        result
    }
}
