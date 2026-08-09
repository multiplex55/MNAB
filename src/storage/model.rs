//! Primitive database records.  Keeping these separate makes the trust boundary explicit: rows
//! are unvalidated, while values in `domain` have passed mapping and aggregate validation.

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BudgetRow {
    pub id: String,
    pub name: String,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountRow {
    pub id: String,
    pub budget_id: String,
    pub name: String,
    pub account_type: String,
    pub closed: i64,
    pub note: Option<String>,
    pub sort_order: i64,
    pub favorite: i64,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransactionRow {
    pub id: String,
    pub budget_id: String,
    pub account_id: String,
    pub date: String,
    pub payee_id: Option<String>,
    pub amount_cents: i64,
    pub memo: Option<String>,
    pub clearance: String,
    pub approval: String,
    pub archived: i64,
    pub voided: i64,
}

/// Raw rows for the account-centric schema. Strings and integer flags remain untrusted until
/// converted by `mapping`; these types are never part of a UI contract.
macro_rules! primitive_row {
    ($name:ident { $($field:ident : $ty:ty),* $(,)? }) => {
        #[derive(Clone, Debug, Eq, PartialEq)]
        pub struct $name { $(pub $field: $ty),* }
    };
}
primitive_row!(ApplicationMetadataRow {
    key: String,
    value: String,
    updated_at: String
});
primitive_row!(AccountGroupRow {
    id: String,
    budget_id: String,
    name: String,
    classification: String,
    sort_order: i64,
    collapsed: i64
});
primitive_row!(CategoryGroupRow {
    id: String,
    budget_id: String,
    name: String,
    sort_order: i64,
    hidden: i64
});
primitive_row!(CategoryRow {
    id: String,
    budget_id: String,
    group_id: String,
    name: String,
    sort_order: i64,
    hidden: i64,
    archived: i64
});
primitive_row!(CategoryGoalRow { id: String, budget_id: String, category_id: String, account_id: Option<String>, goal_type: String, amount: Option<i64>, due_date: Option<String> });
primitive_row!(PayeeRow {
    id: String,
    budget_id: String,
    name: String,
    archived: i64,
    hidden: i64
});
primitive_row!(MerchantRuleRow { id: String, budget_id: String, account_id: Option<String>, pattern: String, match_type: String, payee_id: Option<String>, category_id: Option<String>, priority: i64, enabled: i64 });
primitive_row!(TransactionSplitRow { id: String, budget_id: String, transaction_id: String, category_id: String, memo: Option<String>, amount: i64, sort_order: i64 });
primitive_row!(TransferRow {
    id: String,
    budget_id: String,
    source_transaction_id: String,
    destination_transaction_id: String
});
primitive_row!(ScheduledTransactionRow {
    id: String,
    budget_id: String,
    account_id: String,
    start_date: String,
    recurrence: String,
    amount: i64,
    active: i64
});
primitive_row!(ScheduledOccurrenceRow { id: String, budget_id: String, schedule_id: String, occurrence_date: String, amount: i64, disposition: String, transaction_id: Option<String> });
primitive_row!(ReconciliationRow {
    id: String,
    budget_id: String,
    account_id: String,
    statement_date: String,
    ending_balance: i64,
    state: String,
    difference: i64
});
primitive_row!(ReconciliationChangeRow { id: i64, reconciliation_id: String, budget_id: String, transaction_id: String, operation: String, before_snapshot: Option<String>, after_snapshot: Option<String> });
primitive_row!(ImportSourceRow {
    id: String,
    budget_id: String,
    account_id: String,
    source_identifier: String,
    archive_status: String
});
primitive_row!(ImportBatchRow {
    id: String,
    budget_id: String,
    account_id: String,
    state: String,
    created_at: String
});
primitive_row!(StagedImportCandidateRow {
    id: String,
    budget_id: String,
    batch_id: String,
    normalized_fingerprint: String,
    transaction_date: String,
    amount: i64,
    duplicate_class: String,
    review_decision: String
});
primitive_row!(ImportDecisionRow { candidate_id: String, budget_id: String, decision: String, transaction_id: Option<String>, decided_at: String });
primitive_row!(ImportIdentityRow {
    id: String,
    budget_id: String,
    account_id: String,
    transaction_id: String,
    normalized_fingerprint: String
});
primitive_row!(ApplicationFailureRow { id: String, budget_id: Option<String>, operation: String, summary: String, detail: Option<String>, occurred_at: String, dismissed_at: Option<String> });

/// Values accepted by SQL statements; money is always represented in minor units.
pub trait ToSqlModel {
    type Model;
    fn to_sql_model(&self) -> Self::Model;
}
