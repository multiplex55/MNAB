-- One immutable managed payment category per on-budget credit-card account.
CREATE TABLE credit_card_payment_categories (
    account_id TEXT PRIMARY KEY,
    category_id TEXT NOT NULL UNIQUE,
    FOREIGN KEY(account_id) REFERENCES accounts(id),
    FOREIGN KEY(category_id) REFERENCES categories(id)
);
