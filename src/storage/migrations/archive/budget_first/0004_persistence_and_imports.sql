-- Persist the complete domain model and the import review/audit workflow.
-- Accounts must be rebuilt because SQLite cannot extend their account_type CHECK constraint.
PRAGMA legacy_alter_table = ON;
ALTER TABLE accounts RENAME TO accounts_v3;
CREATE TABLE accounts (
    id TEXT PRIMARY KEY,
    budget_id TEXT NOT NULL,
    name TEXT NOT NULL,
    account_type TEXT NOT NULL CHECK(account_type IN ('checking','savings','cash','credit_card','loan','asset','liability','investment')),
    sort_order INTEGER NOT NULL,
    closed INTEGER NOT NULL DEFAULT 0 CHECK(closed IN (0,1)),
    note TEXT,
    favorite INTEGER NOT NULL DEFAULT 0 CHECK(favorite IN (0,1)),
    created_at TEXT NOT NULL,
    modified_at TEXT NOT NULL,
    UNIQUE(id,budget_id), UNIQUE(budget_id,name),
    FOREIGN KEY(budget_id) REFERENCES budgets(id)
);
INSERT INTO accounts(id,budget_id,name,account_type,sort_order,closed,created_at,modified_at)
SELECT id,budget_id,name,account_type,sort_order,closed,created_at,modified_at FROM accounts_v3;
DROP TABLE accounts_v3;

ALTER TABLE payees ADD COLUMN hidden INTEGER NOT NULL DEFAULT 0 CHECK(hidden IN (0,1));
ALTER TABLE payees ADD COLUMN default_category_id TEXT REFERENCES categories(id);
ALTER TABLE payees ADD COLUMN last_used_category_id TEXT REFERENCES categories(id);
ALTER TABLE transactions ADD COLUMN voided INTEGER NOT NULL DEFAULT 0 CHECK(voided IN (0,1));

-- Targets are reconstructed so invalid combinations cannot be persisted. Amount is nullable only
-- for a credit-card payoff target and recurrence belongs only to an upcoming expense.
ALTER TABLE targets RENAME TO targets_v3;
CREATE TABLE targets (
    id TEXT PRIMARY KEY,
    budget_id TEXT NOT NULL,
    category_id TEXT NOT NULL,
    account_id TEXT,
    target_type TEXT NOT NULL CHECK(target_type IN ('balance_amount','balance_by_date','fixed_monthly_savings','refill_to_amount','upcoming_expense','credit_card_payoff_by_date')),
    amount INTEGER,
    due_date TEXT,
    recurrence TEXT NOT NULL DEFAULT 'none' CHECK(recurrence IN ('none','monthly','yearly')),
    created_at TEXT NOT NULL,
    modified_at TEXT NOT NULL,
    UNIQUE(id,budget_id),
    CHECK((target_type='credit_card_payoff_by_date' AND account_id IS NOT NULL AND amount IS NULL AND due_date IS NOT NULL AND recurrence='none') OR
          (target_type='upcoming_expense' AND account_id IS NULL AND amount IS NOT NULL AND due_date IS NOT NULL) OR
          (target_type='balance_by_date' AND account_id IS NULL AND amount IS NOT NULL AND due_date IS NOT NULL AND recurrence='none') OR
          (target_type IN ('balance_amount','fixed_monthly_savings','refill_to_amount') AND account_id IS NULL AND amount IS NOT NULL AND due_date IS NULL AND recurrence='none')),
    FOREIGN KEY(category_id,budget_id) REFERENCES categories(id,budget_id),
    FOREIGN KEY(account_id,budget_id) REFERENCES accounts(id,budget_id)
);
INSERT INTO targets(id,budget_id,category_id,target_type,amount,due_date,created_at,modified_at)
SELECT id,budget_id,category_id,
       CASE target_type WHEN 'balance' THEN 'balance_amount' WHEN 'monthly' THEN 'fixed_monthly_savings' ELSE target_type END,
       amount,target_month,created_at,modified_at FROM targets_v3;
DROP TABLE targets_v3;

ALTER TABLE scheduled_transactions RENAME TO scheduled_transactions_v3;
CREATE TABLE scheduled_transactions (
    id TEXT PRIMARY KEY, budget_id TEXT NOT NULL, account_id TEXT NOT NULL,
    payee_id TEXT, category_id TEXT, start_date TEXT NOT NULL,
    recurrence TEXT NOT NULL CHECK(recurrence IN ('daily','weekly','every_two_weeks','monthly','yearly','custom_days')),
    custom_interval_days INTEGER,
    end_date TEXT, amount INTEGER NOT NULL, memo TEXT, sort_order INTEGER NOT NULL,
    active INTEGER NOT NULL DEFAULT 1 CHECK(active IN (0,1)), UNIQUE(id,budget_id),
    CHECK((recurrence='custom_days' AND custom_interval_days>0) OR (recurrence<>'custom_days' AND custom_interval_days IS NULL)),
    CHECK(end_date IS NULL OR end_date>=start_date),
    FOREIGN KEY(account_id,budget_id) REFERENCES accounts(id,budget_id),
    FOREIGN KEY(payee_id,budget_id) REFERENCES payees(id,budget_id),
    FOREIGN KEY(category_id,budget_id) REFERENCES categories(id,budget_id)
);
INSERT INTO scheduled_transactions(id,budget_id,account_id,payee_id,category_id,start_date,recurrence,amount,memo,sort_order,active)
SELECT id,budget_id,account_id,payee_id,category_id,next_date,
       CASE frequency WHEN 'biweekly' THEN 'every_two_weeks' ELSE frequency END,
       amount,memo,sort_order,active FROM scheduled_transactions_v3;
DROP TABLE scheduled_transactions_v3;

CREATE TABLE scheduled_occurrences (
    id TEXT PRIMARY KEY,
    budget_id TEXT NOT NULL,
    schedule_id TEXT NOT NULL,
    sequence INTEGER NOT NULL CHECK(sequence>=0),
    occurrence_date TEXT NOT NULL,
    amount INTEGER NOT NULL,
    payee_id TEXT,
    category_id TEXT,
    disposition TEXT NOT NULL DEFAULT 'pending' CHECK(disposition IN ('pending','skipped','dismissed','entered')),
    transaction_id TEXT,
    UNIQUE(schedule_id,sequence),
    CHECK((disposition='entered' AND transaction_id IS NOT NULL) OR (disposition<>'entered' AND transaction_id IS NULL)),
    FOREIGN KEY(schedule_id,budget_id) REFERENCES scheduled_transactions(id,budget_id) ON DELETE CASCADE,
    FOREIGN KEY(transaction_id,budget_id) REFERENCES transactions(id,budget_id),
    FOREIGN KEY(payee_id,budget_id) REFERENCES payees(id,budget_id),
    FOREIGN KEY(category_id,budget_id) REFERENCES categories(id,budget_id)
);

-- Import identity is intentionally separate from the core transaction row.
CREATE TABLE import_sources (
    id TEXT PRIMARY KEY, budget_id TEXT NOT NULL, account_id TEXT NOT NULL,
    source_identifier TEXT NOT NULL, display_name TEXT, archive_status TEXT NOT NULL DEFAULT 'active'
      CHECK(archive_status IN ('active','archived','archive_failed')),
    archive_retry_count INTEGER NOT NULL DEFAULT 0 CHECK(archive_retry_count>=0),
    archive_retry_at TEXT, archive_error TEXT, created_at TEXT NOT NULL, modified_at TEXT NOT NULL,
    UNIQUE(account_id,source_identifier), UNIQUE(id,budget_id),
    FOREIGN KEY(account_id,budget_id) REFERENCES accounts(id,budget_id)
);
CREATE TABLE import_identities (
    id TEXT PRIMARY KEY, budget_id TEXT NOT NULL, account_id TEXT NOT NULL, transaction_id TEXT NOT NULL,
    source_id TEXT, source_record_id TEXT, fitid TEXT, normalized_fingerprint TEXT NOT NULL, created_at TEXT NOT NULL,
    UNIQUE(transaction_id), UNIQUE(account_id,fitid), UNIQUE(account_id,source_id,source_record_id),
    CHECK(normalized_fingerprint<>''),
    CHECK(fitid IS NOT NULL OR (source_id IS NOT NULL AND source_record_id IS NOT NULL)),
    FOREIGN KEY(transaction_id,budget_id) REFERENCES transactions(id,budget_id) ON DELETE CASCADE,
    FOREIGN KEY(account_id,budget_id) REFERENCES accounts(id,budget_id),
    FOREIGN KEY(source_id,budget_id) REFERENCES import_sources(id,budget_id)
);
CREATE TABLE staged_import_candidates (
    id TEXT PRIMARY KEY, budget_id TEXT NOT NULL, batch_id TEXT NOT NULL, source_id TEXT,
    source_record_id TEXT, fitid TEXT, normalized_fingerprint TEXT NOT NULL,
    transaction_date TEXT NOT NULL, payee_text TEXT, memo TEXT, amount INTEGER NOT NULL,
    sort_order INTEGER NOT NULL, UNIQUE(batch_id,sort_order), CHECK(normalized_fingerprint<>''),
    FOREIGN KEY(batch_id,budget_id) REFERENCES import_batches(id,budget_id) ON DELETE CASCADE,
    FOREIGN KEY(source_id,budget_id) REFERENCES import_sources(id,budget_id)
);
CREATE TABLE import_decisions (
    candidate_id TEXT PRIMARY KEY, budget_id TEXT NOT NULL,
    decision TEXT NOT NULL CHECK(decision IN ('accepted','rejected','duplicate','manual_match')),
    transaction_id TEXT, decided_at TEXT NOT NULL,
    CHECK((decision IN ('accepted','manual_match') AND transaction_id IS NOT NULL) OR (decision IN ('rejected','duplicate') AND transaction_id IS NULL)),
    FOREIGN KEY(candidate_id) REFERENCES staged_import_candidates(id) ON DELETE CASCADE,
    FOREIGN KEY(transaction_id,budget_id) REFERENCES transactions(id,budget_id)
);
CREATE TABLE import_manual_matches (
    candidate_id TEXT PRIMARY KEY, budget_id TEXT NOT NULL, transaction_id TEXT NOT NULL UNIQUE,
    matched_at TEXT NOT NULL,
    FOREIGN KEY(candidate_id) REFERENCES staged_import_candidates(id) ON DELETE CASCADE,
    FOREIGN KEY(transaction_id,budget_id) REFERENCES transactions(id,budget_id)
);

-- Replace the original weak, single-column managed-category foreign keys with budget-safe ones.
ALTER TABLE credit_card_payment_categories RENAME TO credit_card_payment_categories_v3;
CREATE TABLE credit_card_payment_categories (
    account_id TEXT PRIMARY KEY, budget_id TEXT NOT NULL, category_id TEXT NOT NULL,
    UNIQUE(category_id), UNIQUE(account_id,budget_id),
    FOREIGN KEY(account_id,budget_id) REFERENCES accounts(id,budget_id) ON DELETE CASCADE,
    FOREIGN KEY(category_id,budget_id) REFERENCES categories(id,budget_id)
);
INSERT INTO credit_card_payment_categories(account_id,budget_id,category_id)
SELECT m.account_id,a.budget_id,m.category_id FROM credit_card_payment_categories_v3 m JOIN accounts a ON a.id=m.account_id;
DROP TABLE credit_card_payment_categories_v3;

CREATE INDEX idx_accounts_budget_order_v4 ON accounts(budget_id,sort_order,id);
CREATE INDEX idx_transactions_register_page ON transactions(account_id,archived,transaction_date DESC,id DESC);
CREATE INDEX idx_transactions_inbox_source ON transactions(account_id,approval_state,import_batch_id,transaction_date,id);
CREATE INDEX idx_transactions_month_category ON transactions(budget_id,transaction_date,category_id,archived);
CREATE INDEX idx_assignments_month_category ON budget_assignments(budget_id,budget_month,category_id);
CREATE INDEX idx_reconciliation_lookup_v4 ON reconciliation_transactions(reconciliation_id,transaction_id);
CREATE INDEX idx_import_identity_fitid ON import_identities(account_id,fitid) WHERE fitid IS NOT NULL;
CREATE INDEX idx_import_identity_fingerprint ON import_identities(account_id,normalized_fingerprint);
CREATE INDEX idx_staged_import_fingerprint ON staged_import_candidates(budget_id,normalized_fingerprint);
CREATE INDEX idx_staged_import_source ON staged_import_candidates(source_id,source_record_id);
CREATE INDEX idx_schedule_occurrence_lookup ON scheduled_occurrences(schedule_id,disposition,occurrence_date,sequence);
PRAGMA legacy_alter_table = OFF;
