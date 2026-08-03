use super::{
    csv_mapping::{AmountColumns, CsvMapping, validate},
    source::{ImportError, ImportedStatement, ImportedTransaction, SourceLocation},
};
use crate::domain::{ImportRounding, Money};
use std::collections::BTreeMap;
use time::{Date, format_description};

pub const MAX_FILE_SIZE: usize = 16 * 1024 * 1024;
pub const MAX_TRANSACTIONS: usize = 100_000;

#[derive(Clone, Debug)]
pub struct CsvOptions {
    pub delimiter: u8,
    pub date_format: String,
    pub mapping: CsvMapping,
}

pub fn detect_delimiter(bytes: &[u8]) -> Result<u8, ImportError> {
    let sample = std::str::from_utf8(bytes).map_err(|e| ImportError::Decode {
        offset: e.valid_up_to(),
        message: "CSV must be UTF-8".into(),
    })?;
    let line = sample
        .lines()
        .find(|l| !l.trim().is_empty())
        .ok_or_else(|| ImportError::Structure("empty delimited file".into()))?;
    [b',', b'\t', b';']
        .into_iter()
        .max_by_key(|d| line.as_bytes().iter().filter(|b| **b == *d).count())
        .filter(|d| line.as_bytes().contains(d))
        .ok_or_else(|| ImportError::Structure("no plausible delimiter".into()))
}

pub fn headers(bytes: &[u8], delimiter: u8) -> Result<Vec<String>, ImportError> {
    let mut reader = csv::ReaderBuilder::new()
        .delimiter(delimiter)
        .from_reader(bytes);
    reader
        .headers()
        .map(|h| h.iter().map(str::trim).map(str::to_owned).collect())
        .map_err(|e| csv_error(e, 1))
}

pub fn parse(bytes: &[u8], options: &CsvOptions) -> Result<ImportedStatement, ImportError> {
    if bytes.len() > MAX_FILE_SIZE {
        return Err(ImportError::SizeLimit {
            limit: MAX_FILE_SIZE,
        });
    }
    let mut reader = csv::ReaderBuilder::new()
        .delimiter(options.delimiter)
        .from_reader(bytes);
    let headers: Vec<String> = reader
        .headers()
        .map_err(|e| csv_error(e, 1))?
        .iter()
        .map(str::trim)
        .map(str::to_owned)
        .collect();
    validate(&options.mapping, &headers).map_err(|e| ImportError::Field {
        location: "CSV mapping".into(),
        field: "columns".into(),
        message: e.to_string(),
    })?;
    let index = |name: &str| {
        headers
            .iter()
            .position(|h| h == name)
            .expect("validated column")
    };
    let date_format = format_description::parse_owned::<2>(&options.date_format).map_err(|e| {
        ImportError::Field {
            location: "CSV mapping".into(),
            field: "date format".into(),
            message: e.to_string(),
        }
    })?;
    let mut transactions = vec![];
    for (offset, result) in reader.records().enumerate() {
        let row = offset as u64 + 2;
        let record = result.map_err(|e| csv_error(e, row))?;
        if transactions.len() >= MAX_TRANSACTIONS {
            return Err(ImportError::TransactionLimit {
                limit: MAX_TRANSACTIONS,
            });
        }
        let get = |name: &str| record.get(index(name)).unwrap_or("").trim();
        let date_text = get(&options.mapping.date);
        let posted_date = Date::parse(date_text, &date_format)
            .map_err(|e| field(row, &options.mapping.date, e.to_string()))?;
        let amount = match &options.mapping.amount {
            AmountColumns::Signed { amount } => money(get(amount), row, amount)?,
            AmountColumns::DebitCredit { debit, credit } => {
                let (d, c) = (get(debit), get(credit));
                if !d.is_empty() && !c.is_empty() {
                    return Err(field(
                        row,
                        "amount",
                        "debit and credit cannot both have values".into(),
                    ));
                }
                if d.is_empty() && c.is_empty() {
                    return Err(field(
                        row,
                        "amount",
                        "debit and credit are both blank".into(),
                    ));
                }
                if d.is_empty() {
                    money(c, row, credit)?
                } else {
                    money(d, row, debit)?
                        .checked_neg()
                        .map_err(|e| field(row, debit, e.to_string()))?
                }
            }
        };
        let optional = |name: &Option<String>| {
            name.as_ref()
                .map(|n| get(n))
                .filter(|v| !v.is_empty())
                .map(str::to_owned)
        };
        let description = optional(&options.mapping.description);
        let payee = optional(&options.mapping.payee).or_else(|| description.clone());
        let mut raw_fields = BTreeMap::new();
        for (h, v) in headers.iter().zip(record.iter()) {
            raw_fields.insert(h.clone(), v.to_owned());
        }
        transactions.push(ImportedTransaction {
            posted_date,
            authorized_date: None,
            amount,
            payee,
            memo: optional(&options.mapping.memo),
            fitid: None,
            check_number: optional(&options.mapping.check_number),
            transaction_type: None,
            source_account: None,
            raw_fields,
            location: SourceLocation::CsvRow(row),
        });
    }
    Ok(ImportedStatement {
        currency: None,
        account: None,
        start_date: None,
        end_date: None,
        ledger_balance: None,
        available_balance: None,
        transactions,
    })
}
fn money(value: &str, row: u64, name: &str) -> Result<Money, ImportError> {
    Money::parse_import(value, ImportRounding::HalfAwayFromZero)
        .map_err(|e| field(row, name, e.to_string()))
}
fn field(row: u64, name: &str, message: String) -> ImportError {
    ImportError::Field {
        location: format!("CSV row {row}"),
        field: name.into(),
        message,
    }
}
fn csv_error(error: csv::Error, fallback: u64) -> ImportError {
    let row = error.position().map_or(fallback, csv::Position::line);
    field(row, "record", error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::importing::csv_mapping::*;
    fn options(delimiter: u8, amount: AmountColumns) -> CsvOptions {
        CsvOptions {
            delimiter,
            date_format: "[year]-[month]-[day]".into(),
            mapping: CsvMapping {
                date: "Date".into(),
                description: None,
                payee: Some("Payee".into()),
                memo: Some("Memo".into()),
                check_number: None,
                amount,
            },
        }
    }
    #[test]
    fn quoted_multiline_and_signed_amount() {
        let data = b"Date,Payee,Memo,Amount\n2026-01-02,Store,\"one,\ntwo\",($1.25)\n";
        let s = parse(
            data,
            &options(
                b',',
                AmountColumns::Signed {
                    amount: "Amount".into(),
                },
            ),
        )
        .unwrap();
        assert_eq!(s.transactions[0].amount.minor_units(), -125);
        assert_eq!(s.transactions[0].memo.as_deref(), Some("one,\ntwo"));
    }
    #[test]
    fn detects_common_delimiters() {
        assert_eq!(
            detect_delimiter(b"Date\tPayee\tAmount\n1\ta\t2").unwrap(),
            b'\t'
        );
        assert_eq!(detect_delimiter(b"Date;Payee;Amount\n1;a;2").unwrap(), b';');
    }
    #[test]
    fn rejects_dual_debit_credit() {
        let data = b"Date,Payee,Memo,Debit,Credit\n2026-01-02,X,,1,2\n";
        let e = parse(
            data,
            &options(
                b',',
                AmountColumns::DebitCredit {
                    debit: "Debit".into(),
                    credit: "Credit".into(),
                },
            ),
        )
        .unwrap_err()
        .to_string();
        assert!(e.contains("CSV row 2"));
    }
}
