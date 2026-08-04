//! The single conversion boundary between SQLite primitives and validated domain values.
use super::{model::*, repository::RowConversionError};
use crate::domain::*;
use time::{Date, format_description::well_known::Iso8601};
use uuid::Uuid;

fn bad(table: &'static str, id: &str, reason: &'static str) -> RowConversionError {
    RowConversionError::new(table, id, reason)
}
pub fn uuid(text: &str, table: &'static str, id: &str) -> Result<Uuid, RowConversionError> {
    Uuid::parse_str(text).map_err(|_| bad(table, id, "invalid identifier"))
}
pub fn date(text: &str, table: &'static str, id: &str) -> Result<Date, RowConversionError> {
    Date::parse(text, &Iso8601::DATE).map_err(|_| bad(table, id, "invalid date"))
}
pub const fn money(cents: i64) -> Money {
    Money::from_minor_units(cents)
}
fn boolean(value: i64, table: &'static str, id: &str) -> Result<bool, RowConversionError> {
    match value {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(bad(table, id, "invalid boolean")),
    }
}
pub fn account_type(value: &str, id: &str) -> Result<AccountType, RowConversionError> {
    match value {
        "checking" => Ok(AccountType::Checking),
        "savings" => Ok(AccountType::Savings),
        "cash" => Ok(AccountType::Cash),
        "credit_card" => Ok(AccountType::CreditCard),
        "loan" => Ok(AccountType::Loan),
        "asset" => Ok(AccountType::Asset),
        "liability" => Ok(AccountType::Liability),
        "investment" => Ok(AccountType::Investment),
        _ => Err(bad("accounts", id, "invalid account type")),
    }
}
pub fn clearance(value: &str, id: &str) -> Result<Clearance, RowConversionError> {
    match value {
        "uncleared" => Ok(Clearance::Uncleared),
        "cleared" => Ok(Clearance::Cleared),
        "reconciled" => Ok(Clearance::Reconciled),
        _ => Err(bad("transactions", id, "invalid clearance")),
    }
}
pub fn approval(value: &str, id: &str) -> Result<Approval, RowConversionError> {
    match value {
        "unapproved" => Ok(Approval::Unapproved),
        "approved" => Ok(Approval::Approved),
        _ => Err(bad("transactions", id, "invalid approval")),
    }
}

impl TryFrom<BudgetRow> for Budget {
    type Error = RowConversionError;
    fn try_from(r: BudgetRow) -> Result<Self, Self::Error> {
        Ok(Self {
            id: BudgetId::from_uuid(uuid(&r.id, "budgets", &r.id)?),
            name: r.name,
        })
    }
}
impl TryFrom<AccountRow> for Account {
    type Error = RowConversionError;
    fn try_from(r: AccountRow) -> Result<Self, Self::Error> {
        let id = AccountId::from_uuid(uuid(&r.id, "accounts", &r.id)?);
        Ok(Self {
            id,
            budget_id: BudgetId::from_uuid(uuid(&r.budget_id, "accounts", &r.id)?),
            name: r.name,
            account_type: account_type(&r.account_type, &r.id)?,
            closed: boolean(r.closed, "accounts", &r.id)?,
            note: r.note,
            sort_order: r.sort_order,
            favorite: boolean(r.favorite, "accounts", &r.id)?,
        })
    }
}
impl ToSqlModel for Budget {
    type Model = BudgetRow;
    fn to_sql_model(&self) -> BudgetRow {
        BudgetRow {
            id: self.id.to_string(),
            name: self.name.clone(),
        }
    }
}
impl ToSqlModel for Account {
    type Model = AccountRow;
    fn to_sql_model(&self) -> AccountRow {
        AccountRow {
            id: self.id.to_string(),
            budget_id: self.budget_id.to_string(),
            name: self.name.clone(),
            account_type: match self.account_type {
                AccountType::Checking => "checking",
                AccountType::Savings => "savings",
                AccountType::Cash => "cash",
                AccountType::CreditCard => "credit_card",
                AccountType::Loan => "loan",
                AccountType::Asset => "asset",
                AccountType::Liability => "liability",
                AccountType::Investment => "investment",
            }
            .into(),
            closed: i64::from(self.closed),
            note: self.note.clone(),
            sort_order: self.sort_order,
            favorite: i64::from(self.favorite),
        }
    }
}

pub fn validate_transaction(value: &Transaction) -> Result<(), RowConversionError> {
    value.validate().map_err(|_| {
        bad(
            "transactions",
            &value.id.to_string(),
            "invalid split aggregate",
        )
    })
}
pub fn validate_transfer_pair(
    left: &Transaction,
    right: &Transaction,
) -> Result<(), RowConversionError> {
    match (&left.body, &right.body) {
        (
            TransactionBody::Transfer {
                transfer_id: a,
                other_account_id: ao,
                other_amount: am,
            },
            TransactionBody::Transfer {
                transfer_id: b,
                other_account_id: bo,
                other_amount: bm,
            },
        ) if a == b
            && *ao == right.account_id
            && *bo == left.account_id
            && *am == right.amount
            && *bm == left.amount
            && left.amount.checked_neg().ok() == Some(right.amount) =>
        {
            Ok(())
        }
        _ => Err(bad(
            "transactions",
            &left.id.to_string(),
            "invalid transfer pair",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::model::ToSqlModel;
    use time::macros::date;

    #[test]
    fn primitive_and_enum_conversions_are_checked() {
        let id = Uuid::new_v4();
        assert_eq!(uuid(&id.to_string(), "x", "record").unwrap(), id);
        assert_eq!(
            date("2026-08-03", "x", "record").unwrap(),
            date!(2026 - 08 - 03)
        );
        assert_eq!(money(-123).minor_units(), -123);
        assert_eq!(
            account_type("credit_card", "record").unwrap(),
            AccountType::CreditCard
        );
        assert_eq!(
            clearance("reconciled", "record").unwrap(),
            Clearance::Reconciled
        );
        assert_eq!(approval("approved", "record").unwrap(), Approval::Approved);
        for error in [
            uuid("not an id", "transactions", "safe-id").unwrap_err(),
            date("bad", "transactions", "safe-id").unwrap_err(),
            account_type("bad", "safe-id").unwrap_err(),
        ] {
            let text = error.to_string();
            assert!(text.contains("safe-id"));
            assert!(!text.contains("memo") && !text.contains("payee") && !text.contains("$"));
        }
    }

    #[test]
    fn complete_account_round_trip() {
        let mut account = Account::new(BudgetId::new(), "Checking", AccountType::Checking);
        account.note = Some("private".into());
        account.closed = true;
        account.favorite = true;
        account.sort_order = 7;
        let restored = Account::try_from(account.to_sql_model()).unwrap();
        assert_eq!(restored, account);
    }

    fn transaction(account: AccountId, amount: i64, body: TransactionBody) -> Transaction {
        Transaction {
            id: TransactionId::new(),
            budget_id: BudgetId::new(),
            account_id: account,
            date: TransactionDate(date!(2026 - 08 - 03)),
            payee_id: None,
            amount: Money::from_minor_units(amount),
            memo: None,
            clearance: Clearance::Uncleared,
            approval: Approval::Approved,
            body,
            archived: false,
            voided: false,
        }
    }
    #[test]
    fn malformed_split_and_transfer_aggregates_are_rejected() {
        let category = CategoryId::new();
        let invalid = transaction(
            AccountId::new(),
            10,
            TransactionBody::Split {
                lines: vec![
                    Subtransaction {
                        category_id: category,
                        amount: Money::from_minor_units(9),
                        memo: None,
                    },
                    Subtransaction {
                        category_id: category,
                        amount: Money::ZERO,
                        memo: None,
                    },
                ],
            },
        );
        assert!(validate_transaction(&invalid).is_err());
        let left = transaction(
            AccountId::new(),
            -10,
            TransactionBody::OpeningBalance { category_id: None },
        );
        let right = transaction(
            AccountId::new(),
            10,
            TransactionBody::OpeningBalance { category_id: None },
        );
        assert!(validate_transfer_pair(&left, &right).is_err());
    }
}
