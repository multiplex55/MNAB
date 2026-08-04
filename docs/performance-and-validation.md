# Performance and validation acceptance criteria

These are deterministic payload and work bounds, not claims of unlimited capacity.
CI tests should assert these limits and stable ordering; wall-clock thresholds are
intentionally excluded because shared runners make them flaky.

## Query and UI bounds

- Register queries use `(transaction_date, id)` keyset cursors and return at most
  **200 rows**. Deep navigation must not use SQL `OFFSET`. At most three pages are
  retained by the register UI.
- Global search is debounced for **200 ms** and returns at most **100 results**.
- Inbox count queries are independent of the review detail window. A detail request
  returns at most **50 items**; counts may describe more items than are materialized.
- Import review pages return at most **200 candidates** and schedule/occurrence
  windows return at most **100 items**.
- Report worker responses contain only typed aggregate series. They must never
  contain raw transactions or split rows. Each series is bounded by the requested
  date range and grouping dimensions; exports stream rather than duplicating the
  full payload in application state.
- Virtualized register rendering creates widgets and formatted values only for the
  visible `egui::ScrollArea::show_rows` range. Frame rendering borrows view models;
  it must not clone the ledger or complete application state.

## Cache invalidation

- Account, category, and payee lookup snapshots are keyed by the relevant data
  revision. Unrelated revisions retain them.
- Completed budget-month results are cached by month and source revision. A dated
  transaction or assignment invalidates the earliest affected month and every
  dependent later month, while earlier months remain cached.
- Reports are cached by budget, normalized parameters, report-specific revision.
  A revision change removes stale entries for that budget without evicting other
  budgets.

## Diagnostics and lifecycle

The **quick** suite runs `PRAGMA quick_check`, foreign-key validation, and all MNAB
financial/association checks. It is used for familiar clean opens. The **full**
suite substitutes `PRAGMA integrity_check` and is mandatory on an unfamiliar open,
after an unclean startup, and after repair. Debug mutation-boundary execution is
optional. A failed required suite refuses normal opening; repair runs the full suite
before replacement.

Query-plan tests assert that required named indexes are available and that critical
queries do not perform a transaction-table scan. They deliberately do not assert
the complete plan text, join order, or temporary B-tree choices, which SQLite may
legitimately change between versions.
