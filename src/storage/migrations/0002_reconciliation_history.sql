ALTER TABLE reconciliations RENAME COLUMN balance TO ending_balance;
ALTER TABLE reconciliations ADD COLUMN calculated_cleared_balance INTEGER NOT NULL DEFAULT 0;
ALTER TABLE reconciliations ADD COLUMN difference INTEGER NOT NULL DEFAULT 0;
ALTER TABLE reconciliations ADD COLUMN state TEXT NOT NULL DEFAULT 'completed' CHECK(state IN ('active','completed','potentially_invalid'));
ALTER TABLE reconciliations ADD COLUMN completed_at TEXT;
ALTER TABLE reconciliations ADD COLUMN invalidated_at TEXT;
CREATE TABLE reconciliation_transactions (
    reconciliation_id TEXT NOT NULL,
    budget_id TEXT NOT NULL,
    transaction_id TEXT NOT NULL,
    included_at TEXT NOT NULL,
    PRIMARY KEY(reconciliation_id, transaction_id),
    FOREIGN KEY(reconciliation_id,budget_id) REFERENCES reconciliations(id,budget_id),
    FOREIGN KEY(transaction_id,budget_id) REFERENCES transactions(id,budget_id)
);
CREATE TABLE reconciliation_change_log (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    reconciliation_id TEXT NOT NULL,
    budget_id TEXT NOT NULL,
    transaction_id TEXT NOT NULL,
    operation TEXT NOT NULL CHECK(operation IN ('update','delete')),
    before_snapshot TEXT NOT NULL,
    after_snapshot TEXT,
    changed_at TEXT NOT NULL,
    FOREIGN KEY(reconciliation_id,budget_id) REFERENCES reconciliations(id,budget_id)
);
CREATE INDEX idx_reconciliation_account_date ON reconciliations(account_id,statement_date);
CREATE INDEX idx_reconciliation_transactions_transaction ON reconciliation_transactions(transaction_id);
