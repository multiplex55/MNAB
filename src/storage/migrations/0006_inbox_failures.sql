-- Inbox financial items are projections and must never be stored here.  This
-- table contains only explicitly dismissible, non-financial operation failures.
CREATE TABLE operation_failures (
    id TEXT PRIMARY KEY,
    budget_id TEXT,
    operation TEXT NOT NULL,
    summary TEXT NOT NULL,
    detail TEXT,
    occurred_at TEXT NOT NULL,
    dismissed_at TEXT,
    FOREIGN KEY(budget_id) REFERENCES budgets(id)
);
CREATE INDEX idx_operation_failures_inbox
    ON operation_failures(budget_id, dismissed_at, occurred_at);
