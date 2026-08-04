ALTER TABLE scheduled_transactions ADD COLUMN version INTEGER NOT NULL DEFAULT 1;

-- Stable generated occurrence identity and explicit reconciliation workflow states.
ALTER TABLE scheduled_occurrences ADD COLUMN identity TEXT;
UPDATE scheduled_occurrences
   SET identity = 'schedule:' || schedule_id || ':sequence:' || sequence
 WHERE identity IS NULL;
CREATE UNIQUE INDEX idx_scheduled_occurrences_identity
    ON scheduled_occurrences(schedule_id, identity);

-- SQLite cannot alter CHECK constraints in place. Runtime writes use these
-- additional states for explicit UI workflow; historical databases still accept
-- old rows because active/completed/potentially_invalid are preserved.
PRAGMA writable_schema=ON;
UPDATE sqlite_schema
   SET sql = replace(sql,
       "CHECK(state IN ('active','completed','potentially_invalid'))",
       "CHECK(state IN ('not_reconciling','entering_statement','active','reviewing_adjustment','completing','completed','potentially_invalid'))")
 WHERE type='table' AND name='reconciliations';
PRAGMA writable_schema=OFF;
