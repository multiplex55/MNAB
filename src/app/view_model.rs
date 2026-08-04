//! Application-facing, persistence-free snapshots.
//!
//! These types deliberately contain domain identifiers, cents, dates and small
//! pieces of display text only. Database rows, repositories and connections stop
//! at the query boundary which constructs these values.

use std::collections::BTreeMap;
use std::collections::BTreeSet;

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

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct RegisterCursor {
    pub date: Date,
    pub transaction_id: TransactionId,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RegisterFilterView {
    pub text: String,
    pub from: Option<Date>,
    pub through: Option<Date>,
    pub category_ids: BTreeSet<CategoryId>,
    pub payee_ids: BTreeSet<PayeeId>,
    pub cleared_only: bool,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RegisterRowView {
    pub transaction_id: TransactionId,
    pub date: Date,
    pub payee: String,
    pub category: String,
    pub memo: Option<String>,
    pub amount_cents: i64,
    pub running_balance_cents: i64,
    pub reconciled: bool,
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
    pub account_id: AccountId,
    pub offset: u64,
    pub cursor: Option<RegisterCursor>,
    pub next_cursor: Option<RegisterCursor>,
    pub total_matches: u64,
    pub running_balance_anchor_cents: i64,
    pub rows: Vec<RegisterRowView>,
    pub separators: Vec<ReconciliationSeparatorView>,
    pub filter: RegisterFilterView,
}

pub const MAX_REGISTER_PAGE_SIZE: usize = 200;
impl RegisterPageView {
    /// Enforces deterministic `(date, id)` order and the hard UI page bound.
    pub fn normalize(&mut self) {
        self.rows.sort_by_key(|row| (row.date, row.transaction_id));
        self.rows.truncate(MAX_REGISTER_PAGE_SIZE);
        self.next_cursor = self.rows.last().map(|row| RegisterCursor {
            date: row.date,
            transaction_id: row.transaction_id,
        });
    }
    pub fn continues_after(&self, cursor: RegisterCursor) -> bool {
        self.rows
            .first()
            .is_none_or(|row| (row.date, row.transaction_id) > (cursor.date, cursor.transaction_id))
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
pub struct SearchResultItemView {
    pub transaction_id: TransactionId,
    pub account_id: AccountId,
    pub date: Date,
    pub title: String,
    pub subtitle: String,
    pub amount: DisplayMoney,
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
            date,
            payee: String::new(),
            category: String::new(),
            memo: None,
            amount_cents: 0,
            running_balance_cents: 0,
            reconciled: false,
        }
    }
    fn page(rows: Vec<RegisterRowView>) -> RegisterPageView {
        RegisterPageView {
            version: ViewVersion::default(),
            account_id: AccountId::new(),
            offset: 0,
            cursor: None,
            next_cursor: None,
            total_matches: rows.len() as u64,
            running_balance_anchor_cents: 0,
            rows,
            separators: vec![],
            filter: RegisterFilterView {
                text: String::new(),
                from: None,
                through: None,
                category_ids: BTreeSet::new(),
                payee_ids: BTreeSet::new(),
                cleared_only: false,
            },
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
        let continuation = page(vec![row(high, date!(2026 - 08 - 04))]);
        assert!(continuation.continues_after(RegisterCursor {
            date: date!(2026 - 08 - 04),
            transaction_id: low
        }));
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
