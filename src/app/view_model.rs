//! Application-facing, persistence-free snapshots.
//!
//! These types deliberately contain domain identifiers, cents, dates and small
//! pieces of display text only. Database rows, repositories and connections stop
//! at the query boundary which constructs these values.

use std::collections::BTreeMap;

use time::Date;

use crate::domain::{
    AccountId, BudgetId, BudgetMonth, CategoryGroupId, CategoryId, ImportBatchId, PayeeId,
    ReconciliationId, ScheduledOccurrenceId, ScheduledTransactionId, TargetId, TransactionId,
};

/// A financial value calculated on the storage thread and ready to render.  `text` is deliberately
/// carried with the minor-unit value so widgets never duplicate sign, currency, or rounding rules.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DisplayMoney {
    pub minor_units: i64,
    pub text: String,
}

impl DisplayMoney {
    #[must_use]
    pub fn usd(minor_units: i64) -> Self {
        let sign = if minor_units < 0 { "-" } else { "" };
        let absolute = i128::from(minor_units).abs();
        Self {
            minor_units,
            text: format!("{sign}${}.{:02}", absolute / 100, absolute % 100),
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SortFilterMetadata {
    pub sort_key: String,
    pub descending: bool,
    pub filter_summary: String,
    pub total_before_filter: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ViewVersion {
    pub generation: u64,
    pub revision: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RefreshState {
    Idle,
    Loading,
    Failed(String),
}

/// Keeps the last successful snapshot visible while a refresh is in flight or fails.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Refreshable<T> {
    pub displayed: Option<T>,
    pub refresh: RefreshState,
    requested_revision: u64,
}

impl<T> Default for Refreshable<T> {
    fn default() -> Self {
        Self {
            displayed: None,
            refresh: RefreshState::Idle,
            requested_revision: 0,
        }
    }
}

impl<T> Refreshable<T> {
    pub fn begin(&mut self) -> u64 {
        self.requested_revision = self.requested_revision.saturating_add(1);
        self.refresh = RefreshState::Loading;
        self.requested_revision
    }
    pub fn accept(&mut self, revision: u64, value: T) -> bool {
        if revision != self.requested_revision {
            return false;
        }
        self.displayed = Some(value);
        self.refresh = RefreshState::Idle;
        true
    }
    pub fn fail(&mut self, revision: u64, error: impl Into<String>) -> bool {
        if revision != self.requested_revision {
            return false;
        }
        self.refresh = RefreshState::Failed(error.into());
        true
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BudgetHeaderView {
    pub version: ViewVersion,
    pub budget_id: BudgetId,
    pub name: String,
    pub month: BudgetMonth,
    pub ready_to_assign_cents: i64,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountSidebarItem {
    pub id: AccountId,
    pub name: String,
    pub balance_cents: i64,
    pub closed: bool,
    pub tracking: bool,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountSidebarView {
    pub version: ViewVersion,
    pub accounts: Vec<AccountSidebarItem>,
    pub on_budget_total_cents: i64,
    pub tracking_total_cents: i64,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BudgetMonthView {
    pub version: ViewVersion,
    pub month: BudgetMonth,
    pub calculation_revision: u64,
    pub ready_to_assign_cents: i64,
    pub assigned_cents: i64,
    pub activity_cents: i64,
    pub available_cents: i64,
    pub overspending_cents: i64,
    pub rows: Vec<CategoryRowView>,
    pub inspector: Vec<String>,
}

/// Session-only memoization of derived reads. Availability is never writable state: every entry
/// is tagged with the source revision and is replaced or discarded as ledger data changes.
#[derive(Clone, Debug, Default)]
pub struct BudgetMonthCache {
    entries: BTreeMap<BudgetMonth, BudgetMonthView>,
}
impl BudgetMonthCache {
    pub fn get(&self, month: BudgetMonth, version: ViewVersion) -> Option<&BudgetMonthView> {
        self.entries
            .get(&month)
            .filter(|view| view.version == version)
    }
    pub fn insert(&mut self, view: BudgetMonthView) {
        self.entries.insert(view.month, view);
    }
    pub fn invalidate_from(&mut self, month: BudgetMonth) {
        self.entries.retain(|cached, _| *cached < month);
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CategoryRowView {
    pub group_id: CategoryGroupId,
    pub category_id: CategoryId,
    pub group_name: String,
    pub name: String,
    pub group_sort: i64,
    pub category_sort: i64,
    pub group_collapsed: bool,
    pub assigned_cents: i64,
    pub activity_cents: i64,
    pub available_cents: i64,
    pub overspending_cents: i64,
    pub underfunded_cents: i64,
    pub target_id: Option<TargetId>,
    pub target_amount_cents: Option<i64>,
    pub target_remaining_cents: Option<i64>,
    pub target_due_date: Option<String>,
    pub target_status: String,
    pub credit_card_payment: bool,
    pub protected: bool,
    pub hidden: bool,
    pub archived: bool,
    pub inspector: String,
}

/// The identity of a register.  This is the only register scope used by the
/// application, worker protocol, storage query, and widgets.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum RegisterScope {
    Account(AccountId),
    AllTransactions,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct RegisterCursor {
    pub date: Date,
    pub created_at: String,
    pub transaction_id: TransactionId,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RegisterSortDirection {
    Ascending,
    Descending,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RegisterSortField {
    Date,
}
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RegisterFilter {
    pub search: String,
    pub from: Option<Date>,
    pub through: Option<Date>,
    pub category_ids: Vec<CategoryId>,
    pub payee_ids: Vec<PayeeId>,
    pub cleared_state: Option<String>,
    pub approval_state: Option<String>,
    pub minimum_amount_cents: Option<i64>,
    pub maximum_amount_cents: Option<i64>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RegisterRequest {
    pub budget_id: BudgetId,
    pub scope: RegisterScope,
    pub filter: RegisterFilter,
    pub sort_field: RegisterSortField,
    pub sort_direction: RegisterSortDirection,
    pub page_size: usize,
    pub cursor: Option<RegisterCursor>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RegisterRowView {
    pub transaction_id: TransactionId,
    pub account_id: AccountId,
    pub account_name: String,
    pub date: Date,
    pub created_at: String,
    pub payee_id: Option<PayeeId>,
    pub payee_name: String,
    pub category_id: Option<CategoryId>,
    pub category_name: String,
    pub memo: Option<String>,
    pub inflow_cents: i64,
    pub outflow_cents: i64,
    pub cleared_state: String,
    pub approved: bool,
    pub reconciled: bool,
    pub transfer_id: Option<String>,
    pub is_transfer: bool,
    pub split_count: u32,
    pub import_batch_id: Option<ImportBatchId>,
    pub import_source: Option<String>,
    pub review_state: Option<String>,
    pub running_balance_cents: Option<i64>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReconciliationSeparatorView {
    pub reconciliation_id: ReconciliationId,
    pub after: RegisterCursor,
    pub label: String,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RegisterPageView {
    pub version: ViewVersion,
    pub scope: RegisterScope,
    pub request: RegisterRequest,
    pub cursor: Option<RegisterCursor>,
    pub next_cursor: Option<RegisterCursor>,
    pub total_matches: u64,
    pub has_more: bool,
    pub rows: Vec<RegisterRowView>,
    pub separators: Vec<ReconciliationSeparatorView>,
}

pub const MAX_REGISTER_PAGE_SIZE: usize = 200;
impl RegisterPageView {
    /// Enforces deterministic `(date, id)` order and the hard UI page bound.
    pub fn normalize(&mut self) {
        self.rows
            .sort_by_key(|row| (row.date, row.created_at.clone(), row.transaction_id));
        self.rows.truncate(MAX_REGISTER_PAGE_SIZE);
        self.next_cursor = self.rows.last().map(|row| RegisterCursor {
            date: row.date,
            created_at: row.created_at.clone(),
            transaction_id: row.transaction_id,
        });
    }
    pub fn continues_after(&self, cursor: RegisterCursor) -> bool {
        self.rows.first().is_none_or(|row| {
            (row.date, &row.created_at, row.transaction_id)
                > (cursor.date, &cursor.created_at, cursor.transaction_id)
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransactionInspectorView {
    pub version: ViewVersion,
    pub transaction_id: TransactionId,
    pub account_id: AccountId,
    pub date: Date,
    pub payee_id: Option<PayeeId>,
    pub category_id: Option<CategoryId>,
    pub amount_cents: i64,
    pub memo: Option<String>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImportCandidateView {
    pub index: u32,
    pub date: Date,
    pub original_payee: Option<String>,
    pub proposed_payee: Option<String>,
    pub proposed_category: Option<String>,
    pub original_memo: Option<String>,
    pub proposed_memo: Option<String>,
    pub amount_cents: i64,
    pub duplicate_class: String,
    pub duplicate_explanation: Option<String>,
    pub warnings: Vec<String>,
    pub decision: String,
    pub selected: bool,
    pub match_candidate_ids: Vec<TransactionId>,
    pub available_actions: Vec<String>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImportReviewView {
    pub version: ViewVersion,
    pub batch_id: ImportBatchId,
    pub account_id: AccountId,
    pub statement_account: Option<String>,
    pub account_mismatch: Option<String>,
    pub candidates: Vec<ImportCandidateView>,
    pub selected_candidate_count: usize,
    pub bulk_actions: Vec<String>,
    pub can_apply: bool,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReconciliationView {
    pub version: ViewVersion,
    pub reconciliation_id: ReconciliationId,
    pub account_id: AccountId,
    pub statement_date: Date,
    pub ending_balance_cents: i64,
    pub cleared_balance_cents: i64,
    pub difference_cents: i64,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TargetView {
    pub version: ViewVersion,
    pub target_id: TargetId,
    pub category_id: CategoryId,
    pub month: BudgetMonth,
    pub needed_cents: i64,
    pub funded_cents: i64,
    pub progress_basis_points: u16,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScheduleItemView {
    pub schedule_id: ScheduledTransactionId,
    pub next_date: Date,
    pub payee: String,
    pub amount_cents: i64,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScheduleView {
    pub version: ViewVersion,
    pub items: Vec<ScheduleItemView>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReportPointView {
    pub label: String,
    pub income_cents: i64,
    pub expense_cents: i64,
    pub net_cents: i64,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReportView {
    pub version: ViewVersion,
    pub title: String,
    pub points: Vec<ReportPointView>,
    pub total_cents: i64,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InboxSummaryView {
    pub version: ViewVersion,
    pub unapproved_count: u64,
    pub import_count: u64,
    pub scheduled_count: u64,
    pub target_attention_count: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HighlightSpanView {
    pub field: String,
    pub start: usize,
    pub end: usize,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchResultItemView {
    pub transaction_id: TransactionId,
    pub account_id: AccountId,
    pub account: String,
    pub date: Date,
    pub payee: String,
    pub category: String,
    pub memo: String,
    pub amount: DisplayMoney,
    pub approved: bool,
    pub clearance: String,
    pub title: String,
    pub subtitle: String,
    pub highlights: Vec<HighlightSpanView>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchResultsView {
    pub version: ViewVersion,
    pub query: String,
    pub metadata: SortFilterMetadata,
    pub results: Vec<SearchResultItemView>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScheduledOccurrenceView {
    pub occurrence_id: ScheduledOccurrenceId,
    pub schedule_id: ScheduledTransactionId,
    pub date: Date,
    pub payee: String,
    pub amount: DisplayMoney,
    pub safe_status: String,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OccurrencesView {
    pub version: ViewVersion,
    pub through: Date,
    pub occurrences: Vec<ScheduledOccurrenceView>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiagnosticFindingView {
    pub severity: String,
    pub check: String,
    pub entity_reference: Option<String>,
    pub safe_explanation: String,
    pub safe_remediation: String,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiagnosticsView {
    pub version: ViewVersion,
    pub findings: Vec<DiagnosticFindingView>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandOutcomeView {
    pub version: ViewVersion,
    pub summary: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::date;
    fn row(id: TransactionId, date: Date) -> RegisterRowView {
        RegisterRowView {
            transaction_id: id,
            account_id: AccountId::new(),
            account_name: "Account".into(),
            date,
            created_at: "2026-08-04T00:00:00Z".into(),
            payee_id: None,
            payee_name: String::new(),
            category_id: None,
            category_name: String::new(),
            memo: None,
            inflow_cents: 0,
            outflow_cents: 0,
            cleared_state: "uncleared".into(),
            approved: false,
            reconciled: false,
            transfer_id: None,
            is_transfer: false,
            split_count: 0,
            import_batch_id: None,
            import_source: None,
            review_state: None,
            running_balance_cents: Some(0),
        }
    }
    fn page(rows: Vec<RegisterRowView>) -> RegisterPageView {
        let budget_id = BudgetId::new();
        let account = AccountId::new();
        let request = RegisterRequest {
            budget_id,
            scope: RegisterScope::Account(account),
            filter: RegisterFilter::default(),
            sort_field: RegisterSortField::Date,
            sort_direction: RegisterSortDirection::Ascending,
            page_size: 200,
            cursor: None,
        };
        RegisterPageView {
            version: ViewVersion::default(),
            scope: request.scope,
            request,
            cursor: None,
            next_cursor: None,
            total_matches: rows.len() as u64,
            has_more: false,
            rows,
            separators: vec![],
        }
    }
    #[test]
    fn register_order_and_equal_date_continuation_are_stable() {
        let a = TransactionId::new();
        let b = TransactionId::new();
        let (low, high) = if a < b { (a, b) } else { (b, a) };
        let mut p = page(vec![
            row(high, date!(2026 - 08 - 04)),
            row(low, date!(2026 - 08 - 04)),
        ]);
        p.normalize();
        assert_eq!(
            p.rows.iter().map(|r| r.transaction_id).collect::<Vec<_>>(),
            vec![low, high]
        );
        assert!(
            page(vec![row(high, date!(2026 - 08 - 04))]).continues_after(RegisterCursor {
                date: date!(2026 - 08 - 04),
                created_at: "2026-08-04T00:00:00Z".into(),
                transaction_id: low
            })
        );
    }
    #[test]
    fn page_size_is_bounded() {
        let mut p = page(
            (0..250)
                .map(|_| row(TransactionId::new(), date!(2026 - 08 - 04)))
                .collect(),
        );
        p.normalize();
        assert_eq!(p.rows.len(), MAX_REGISTER_PAGE_SIZE);
    }
    #[test]
    fn independent_views_accept_their_own_latest_response() {
        let mut a = Refreshable::default();
        let mut b = Refreshable::default();
        let old = a.begin();
        let latest = a.begin();
        let only = b.begin();
        assert!(!a.accept(old, 1));
        assert!(b.accept(only, 2));
        assert!(a.accept(latest, 3));
    }
    #[test]
    fn failure_preserves_successful_data() {
        let mut v = Refreshable::default();
        let first = v.begin();
        assert!(v.accept(first, 42));
        let refresh = v.begin();
        assert!(v.fail(refresh, "offline"));
        assert_eq!(v.displayed, Some(42));
        assert!(matches!(v.refresh, RefreshState::Failed(_)));
    }
}
