-- Forward-only indexes selected from EXPLAIN QUERY PLAN runs against deterministic
-- large fixtures.  Keep these aligned with the bounded projection queries rather
-- than adding indexes to released migrations.
CREATE INDEX idx_transactions_budget_register_page
    ON transactions(budget_id, archived, transaction_date DESC, id DESC);
CREATE INDEX idx_transactions_budget_report_v7
    ON transactions(budget_id, archived, voided, transaction_date, account_id, category_id);
CREATE INDEX idx_reconciliations_budget_date
    ON reconciliations(budget_id, statement_date DESC, id DESC);
CREATE INDEX idx_scheduled_occurrences_inbox
    ON scheduled_occurrences(budget_id, disposition, occurrence_date, sequence);
CREATE INDEX idx_staged_candidates_batch_review
    ON staged_import_candidates(batch_id, review_decision, sort_order);
CREATE INDEX idx_change_log_report_revision
    ON change_log(budget_id, entity_table, id DESC);
