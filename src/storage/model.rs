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

/// Values accepted by SQL statements; money is always represented in minor units.
pub trait ToSqlModel {
    type Model;
    fn to_sql_model(&self) -> Self::Model;
}
