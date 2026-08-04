-- Durable, restart-safe import staging.  This table deliberately separates workflow
-- state from the legacy import_batches state constraint.
CREATE TABLE import_batch_workflow (
    batch_id TEXT PRIMARY KEY,
    budget_id TEXT NOT NULL,
    state TEXT NOT NULL CHECK(state IN ('selected','parsed_staged','awaiting_review','applied','archive_pending','archived','failed')),
    source_path TEXT, source_checksum TEXT, source_account_json TEXT,
    archive_path TEXT, archive_error TEXT, failure_context TEXT,
    created_at TEXT NOT NULL, modified_at TEXT NOT NULL,
    FOREIGN KEY(batch_id,budget_id) REFERENCES import_batches(id,budget_id) ON DELETE CASCADE
);
CREATE TABLE csv_mapping_presets (
    id TEXT PRIMARY KEY, budget_id TEXT NOT NULL, name TEXT NOT NULL,
    source_signature TEXT NOT NULL, mapping_json TEXT NOT NULL,
    created_at TEXT NOT NULL, modified_at TEXT NOT NULL,
    UNIQUE(budget_id,source_signature), FOREIGN KEY(budget_id) REFERENCES budgets(id)
);
ALTER TABLE staged_import_candidates ADD COLUMN original_json TEXT;
ALTER TABLE staged_import_candidates ADD COLUMN proposed_payee TEXT;
ALTER TABLE staged_import_candidates ADD COLUMN proposed_category_id TEXT;
ALTER TABLE staged_import_candidates ADD COLUMN proposed_memo TEXT;
ALTER TABLE staged_import_candidates ADD COLUMN duplicate_class TEXT NOT NULL DEFAULT 'new'
 CHECK(duplicate_class IN ('new','possible_duplicate','exact_duplicate','possible_manual_match','invalid','ignored'));
ALTER TABLE staged_import_candidates ADD COLUMN duplicate_explanation TEXT;
ALTER TABLE staged_import_candidates ADD COLUMN warnings_json TEXT NOT NULL DEFAULT '[]';
ALTER TABLE staged_import_candidates ADD COLUMN review_decision TEXT NOT NULL DEFAULT 'pending'
 CHECK(review_decision IN ('pending','accept','ignore','match_existing'));
ALTER TABLE staged_import_candidates ADD COLUMN matched_transaction_id TEXT;
ALTER TABLE staged_import_candidates ADD COLUMN exact_duplicate_override INTEGER NOT NULL DEFAULT 0 CHECK(exact_duplicate_override IN (0,1));
CREATE INDEX idx_import_workflow_state ON import_batch_workflow(budget_id,state,modified_at);
