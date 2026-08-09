-- Generalized rule payload. Legacy columns remain intact so old merchant behavior is
-- recoverable; readers use the typed JSON when present and otherwise adapt legacy columns.
ALTER TABLE merchant_rules ADD COLUMN name TEXT NOT NULL DEFAULT '';
ALTER TABLE merchant_rules ADD COLUMN description TEXT NOT NULL DEFAULT '';
ALTER TABLE merchant_rules ADD COLUMN conditions_json TEXT;
ALTER TABLE merchant_rules ADD COLUMN actions_json TEXT;
ALTER TABLE merchant_rules ADD COLUMN match_count INTEGER NOT NULL DEFAULT 0 CHECK(match_count >= 0);
ALTER TABLE merchant_rules ADD COLUMN last_used_date TEXT;
UPDATE merchant_rules SET name = 'Merchant: ' || pattern WHERE name = '';
CREATE INDEX idx_transaction_rules_deterministic
ON merchant_rules(budget_id, enabled DESC, origin ASC, priority DESC,
                  CASE WHEN account_id IS NULL THEN 0 ELSE 1 END DESC, id ASC);
