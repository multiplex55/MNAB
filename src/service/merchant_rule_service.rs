//! Pure, import-only rule evaluation. Transaction and schedule services deliberately do not
//! depend on this module.
use crate::domain::*;
use std::{cmp::Reverse, collections::HashMap};
use thiserror::Error;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImportRuleContext<'a> {
    pub merchant: &'a str,
    pub account_id: AccountId,
    pub account_group_id: Option<AccountGroupId>,
    pub amount_minor_units: i64,
    pub memo: Option<&'a str>,
    pub import_source: &'a str,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RuleProposal {
    pub payee: Option<(PayeeId, String)>,
    pub category: Option<CategoryId>,
    pub memo: Option<String>,
    pub matched_rule_ids: Vec<TransactionRuleId>,
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum RuleEvaluationError {
    #[error("explicit rules {rule_ids:?} have equal precedence and conflict on {field}")]
    ExplicitConflict {
        field: &'static str,
        rule_ids: Vec<TransactionRuleId>,
    },
    #[error("legacy regular expression is invalid or unsupported in rule {rule_id}")]
    InvalidLegacyRegex { rule_id: TransactionRuleId },
}

#[derive(Clone, Debug, Default)]
pub struct MerchantRuleBook {
    pub rules: Vec<TransactionRule>,
}

impl MerchantRuleBook {
    pub fn evaluate(
        &self,
        context: &ImportRuleContext<'_>,
    ) -> Result<RuleProposal, RuleEvaluationError> {
        let mut matching: Vec<_> = self
            .rules
            .iter()
            .filter(|r| {
                r.enabled
                    && (r.origin == MerchantRuleOrigin::Explicit
                        || r.confidence == MerchantConfidence::High)
                    && matches_all(r, context)
            })
            .collect();
        matching.sort_by_key(|r| {
            (
                Reverse(r.origin),
                Reverse(r.priority),
                Reverse(r.account_scope().is_some()),
                Reverse(r.conditions.len()),
                r.id,
            )
        });
        detect_conflicts(&matching)?;
        let mut out = RuleProposal::default();
        for rule in matching {
            let before = out.clone();
            for action in &rule.actions {
                apply_action(&mut out, action);
            }
            if out != before {
                out.matched_rule_ids.push(rule.id);
            }
        }
        Ok(out)
    }

    /// Records only an accepted reviewed outcome. Explicit rules are never rewritten.
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
                && r.account_scope() == Some(account)
                && r.normalized_merchant() == Some(&normalized)
                && r.actions.contains(&RuleAction::SetCategory {
                    category_id: category,
                })
        }) {
            rule.usage_count = rule.usage_count.saturating_add(1);
            rule.last_used_date = Some(date);
            if rule.usage_count >= 3 {
                rule.confidence = MerchantConfidence::High;
            }
            return;
        }
        self.rules.push(TransactionRule {
            id: TransactionRuleId::new(),
            name: format!("Learned: {normalized}"),
            description: String::new(),
            enabled: true,
            priority: 0,
            origin: MerchantRuleOrigin::Learned,
            conditions: vec![
                RuleCondition::Merchant {
                    value: normalized,
                    match_type: TextMatch::Exact,
                },
                RuleCondition::Account(account),
            ],
            actions: vec![RuleAction::SetCategory {
                category_id: category,
            }],
            confidence: MerchantConfidence::Learning,
            usage_count: 1,
            match_count: 0,
            last_used_date: Some(date),
        });
    }
    #[must_use]
    pub fn learned_candidates(&self) -> HashMap<(&str, Option<AccountId>), Vec<&TransactionRule>> {
        let mut result = HashMap::new();
        for r in self
            .rules
            .iter()
            .filter(|r| r.origin == MerchantRuleOrigin::Learned)
        {
            if let Some(m) = r.normalized_merchant() {
                result
                    .entry((m, r.account_scope()))
                    .or_insert_with(Vec::new)
                    .push(r);
            }
        }
        result
    }
}

fn text_match(actual: &str, expected: &str, kind: TextMatch) -> bool {
    match kind {
        TextMatch::Exact => actual == expected,
        TextMatch::Contains => actual.contains(expected),
        TextMatch::Prefix => actual.starts_with(expected),
    }
}
fn matches_all(rule: &TransactionRule, c: &ImportRuleContext<'_>) -> bool {
    let merchant = normalize_merchant(c.merchant);
    rule.conditions.iter().all(|condition| match condition {
        RuleCondition::Merchant { value, match_type } => {
            text_match(&merchant, &normalize_merchant(value), *match_type)
        }
        RuleCondition::LegacyMerchantRegex { .. } => false,
        RuleCondition::Account(id) => *id == c.account_id,
        RuleCondition::AccountGroup(id) => Some(*id) == c.account_group_id,
        RuleCondition::Direction(d) => matches!(
            (d, c.amount_minor_units),
            (TransactionDirection::Inflow, 1..) | (TransactionDirection::Outflow, ..=-1)
        ),
        RuleCondition::AmountExact(v) => *v == c.amount_minor_units,
        RuleCondition::AmountRange { minimum, maximum } => {
            minimum <= &c.amount_minor_units && &c.amount_minor_units <= maximum
        }
        RuleCondition::Memo { value, match_type } => text_match(
            &c.memo.unwrap_or("").to_lowercase(),
            &value.to_lowercase(),
            *match_type,
        ),
        RuleCondition::ImportSource(v) => v.eq_ignore_ascii_case(c.import_source),
    })
}

fn action_field(a: &RuleAction) -> &'static str {
    match a {
        RuleAction::SetPayee { .. } => "payee",
        RuleAction::SetCategory { .. } => "category",
        RuleAction::SetMemo { .. }
        | RuleAction::PrefixMemo { .. }
        | RuleAction::SuffixMemo { .. } => "memo",
    }
}
fn detect_conflicts(rules: &[&TransactionRule]) -> Result<(), RuleEvaluationError> {
    for (i, a) in rules.iter().enumerate() {
        if a.origin != MerchantRuleOrigin::Explicit {
            continue;
        }
        for b in &rules[i + 1..] {
            let same = a.priority == b.priority
                && a.account_scope().is_some() == b.account_scope().is_some()
                && a.conditions.len() == b.conditions.len()
                && b.origin == MerchantRuleOrigin::Explicit;
            if !same {
                continue;
            }
            for aa in &a.actions {
                for ba in &b.actions {
                    if action_field(aa) == action_field(ba) && aa != ba {
                        return Err(RuleEvaluationError::ExplicitConflict {
                            field: action_field(aa),
                            rule_ids: vec![a.id, b.id],
                        });
                    }
                }
            }
        }
    }
    Ok(())
}
fn apply_action(out: &mut RuleProposal, action: &RuleAction) {
    match action {
        RuleAction::SetPayee {
            payee_id,
            display_name_snapshot,
        } if out.payee.is_none() => out.payee = Some((*payee_id, display_name_snapshot.clone())),
        RuleAction::SetCategory { category_id } if out.category.is_none() => {
            out.category = Some(*category_id)
        }
        RuleAction::SetMemo { memo } if out.memo.is_none() => out.memo = Some(memo.clone()),
        RuleAction::PrefixMemo { prefix } => {
            let memo = out.memo.take().unwrap_or_default();
            out.memo = Some(format!("{prefix}{memo}"));
        }
        RuleAction::SuffixMemo { suffix } => {
            let memo = out.memo.take().unwrap_or_default();
            out.memo = Some(format!("{memo}{suffix}"));
        }
        _ => {}
    }
}
