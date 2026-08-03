use super::source::{
    ImportError, ImportedStatement, ImportedTransaction, SourceAccount, SourceLocation,
};
use crate::domain::{ImportRounding, Money};
use encoding_rs::{Encoding, UTF_8, WINDOWS_1252};
use std::collections::BTreeMap;
use time::{Date, Month};

pub const MAX_FILE_SIZE: usize = 16 * 1024 * 1024;
pub const MAX_DEPTH: usize = 64;
pub const MAX_FIELD_LENGTH: usize = 64 * 1024;
pub const MAX_TRANSACTIONS: usize = 100_000;

pub fn parse(bytes: &[u8]) -> Result<ImportedStatement, ImportError> {
    if bytes.len() > MAX_FILE_SIZE {
        return Err(ImportError::SizeLimit {
            limit: MAX_FILE_SIZE,
        });
    }
    let bytes = bytes.strip_suffix(&[0]).map_or(bytes, |_| {
        let end = bytes.iter().rposition(|b| *b != 0).map_or(0, |i| i + 1);
        &bytes[..end]
    });
    let ascii = String::from_utf8_lossy(&bytes[..bytes.len().min(4096)]).to_ascii_uppercase();
    let encoding = declared_encoding(&ascii);
    let (decoded, _, had_errors) = encoding.decode(bytes);
    if had_errors {
        return Err(ImportError::Decode {
            offset: first_decode_error(bytes, encoding),
            message: format!("invalid {} sequence", encoding.name()),
        });
    }
    // Some institutions concatenate an export header twice. Start at the last
    // header's OFX body only when another complete header precedes it.
    let mut text = decoded.trim_end_matches('\0');
    let upper = text.to_ascii_uppercase();
    let ofx_positions: Vec<_> = upper.match_indices("<OFX").map(|(i, _)| i).collect();
    if ofx_positions.len() > 1 {
        text = &text[*ofx_positions.last().unwrap()..];
    } else if let Some(position) = ofx_positions.first() {
        text = &text[*position..];
    }
    if !text.trim_start().to_ascii_uppercase().starts_with("<OFX") {
        return Err(ImportError::Structure("missing OFX root element".into()));
    }
    let family = text.to_ascii_uppercase();
    if !(family.contains("<BANKMSGSRSV1") || family.contains("<CREDITCARDMSGSRSV1"))
        || !(family.contains("<STMTRS") || family.contains("<CCSTMTRS"))
    {
        return Err(ImportError::Structure(
            "OFX does not contain a bank or credit-card statement response".into(),
        ));
    }
    validate_structure(text)?;

    let account_id = value(text, "ACCTID").map(clean);
    let account = account_id.map(|account_id| SourceAccount {
        bank_id: value(text, "BANKID").map(clean),
        account_id,
        account_type: value(text, "ACCTTYPE")
            .map(clean)
            .or_else(|| family.contains("<CCACCTFROM").then(|| "CREDITCARD".into())),
    });
    let blocks = transaction_blocks(text);
    if blocks.len() > MAX_TRANSACTIONS {
        return Err(ImportError::TransactionLimit {
            limit: MAX_TRANSACTIONS,
        });
    }
    let mut transactions = Vec::with_capacity(blocks.len());
    for (offset, block) in blocks.into_iter().enumerate() {
        let index = offset + 1;
        let fitid = value(block, "FITID").map(clean);
        let label = fitid.as_ref().map_or_else(
            || format!("OFX transaction {index}"),
            |f| format!("OFX transaction {index} (FITID {f})"),
        );
        let required = |tag: &str| {
            value(block, tag).ok_or_else(|| ImportError::Field {
                location: label.clone(),
                field: tag.into(),
                message: "missing value".into(),
            })
        };
        let date_text = required("DTPOSTED")?;
        let posted_date = parse_ofx_date(date_text).map_err(|message| ImportError::Field {
            location: label.clone(),
            field: "DTPOSTED".into(),
            message,
        })?;
        let amount_text = required("TRNAMT")?;
        let amount = Money::parse_import(amount_text.trim(), ImportRounding::HalfAwayFromZero)
            .map_err(|e| ImportError::Field {
                location: label.clone(),
                field: "TRNAMT".into(),
                message: e.to_string(),
            })?;
        let mut raw_fields = BTreeMap::new();
        for tag in [
            "TRNTYPE", "DTPOSTED", "DTUSER", "TRNAMT", "FITID", "NAME", "MEMO", "CHECKNUM",
        ] {
            if let Some(v) = value(block, tag) {
                let v = clean(v);
                if v.len() > MAX_FIELD_LENGTH {
                    return Err(ImportError::Field {
                        location: label.clone(),
                        field: tag.into(),
                        message: "field length limit exceeded".into(),
                    });
                }
                raw_fields.insert(tag.into(), v);
            }
        }
        transactions.push(ImportedTransaction {
            posted_date,
            authorized_date: value(block, "DTUSER")
                .map(parse_ofx_date)
                .transpose()
                .map_err(|message| ImportError::Field {
                    location: label.clone(),
                    field: "DTUSER".into(),
                    message,
                })?,
            amount,
            payee: value(block, "NAME").map(clean),
            memo: value(block, "MEMO").map(clean),
            fitid: fitid.clone(),
            check_number: value(block, "CHECKNUM").map(clean),
            transaction_type: value(block, "TRNTYPE").map(clean),
            source_account: account.clone(),
            raw_fields,
            location: SourceLocation::OfxTransaction { index, fitid },
        });
    }
    Ok(ImportedStatement {
        currency: value(text, "CURDEF").map(clean),
        account,
        start_date: value(text, "DTSTART")
            .map(parse_ofx_date)
            .transpose()
            .map_err(|message| field("statement", "DTSTART", message))?,
        end_date: value(text, "DTEND")
            .map(parse_ofx_date)
            .transpose()
            .map_err(|message| field("statement", "DTEND", message))?,
        ledger_balance: optional_money(text, "LEDGERBAL", "BALAMT")?,
        available_balance: optional_money(text, "AVAILBAL", "BALAMT")?,
        transactions,
    })
}

fn declared_encoding(header: &str) -> &'static Encoding {
    let declared = header.lines().find_map(|l| {
        l.split_once(':')
            .filter(|(k, _)| k.trim() == "ENCODING")
            .map(|(_, v)| v.trim())
    });
    match declared {
        Some("1252" | "WINDOWS-1252" | "USASCII" | "ASCII") => WINDOWS_1252,
        Some(name) => Encoding::for_label(name.as_bytes()).unwrap_or(UTF_8),
        None => UTF_8,
    }
}
fn first_decode_error(bytes: &[u8], encoding: &'static Encoding) -> usize {
    if encoding == UTF_8 {
        std::str::from_utf8(bytes)
            .err()
            .map_or(0, |e| e.valid_up_to())
    } else {
        0
    }
}
fn validate_structure(text: &str) -> Result<(), ImportError> {
    let bytes = text.as_bytes();
    let mut depth = 0usize;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'<' {
            if let Some(end) = text[i..].find('>') {
                let token = text[i + 1..i + end].trim();
                let name = token
                    .trim_start_matches('/')
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .trim_end_matches('/')
                    .to_ascii_uppercase();
                if token.starts_with('/') && !is_leaf(&name) {
                    depth = depth.saturating_sub(1);
                } else if !token.starts_with(['?', '!']) && !token.ends_with('/') && !is_leaf(&name)
                {
                    depth += 1;
                }
                if depth > MAX_DEPTH {
                    return Err(ImportError::DepthLimit { limit: MAX_DEPTH });
                }
                i += end;
            } else {
                return Err(ImportError::Structure(format!(
                    "unterminated tag near byte {i}"
                )));
            }
        }
        i += 1;
    }
    Ok(())
}
fn is_leaf(tag: &str) -> bool {
    matches!(
        tag,
        "CODE"
            | "SEVERITY"
            | "MESSAGE"
            | "DTSERVER"
            | "LANGUAGE"
            | "CURDEF"
            | "BANKID"
            | "BRANCHID"
            | "ACCTID"
            | "ACCTTYPE"
            | "ACCTKEY"
            | "DTSTART"
            | "DTEND"
            | "TRNTYPE"
            | "DTPOSTED"
            | "DTUSER"
            | "DTAVAIL"
            | "TRNAMT"
            | "FITID"
            | "CORRECTFITID"
            | "CORRECTACTION"
            | "SRVRTID"
            | "CHECKNUM"
            | "REFNUM"
            | "SIC"
            | "PAYEEID"
            | "NAME"
            | "MEMO"
            | "BALAMT"
            | "DTASOF"
    )
}
fn transaction_blocks(text: &str) -> Vec<&str> {
    let upper = text.to_ascii_uppercase();
    let mut out = vec![];
    let mut cursor = 0;
    while let Some(rel) = upper[cursor..].find("<STMTTRN>") {
        let start = cursor + rel + 9;
        let tail = &upper[start..];
        let end = tail
            .find("</STMTTRN>")
            .or_else(|| tail.find("<STMTTRN>"))
            .unwrap_or(tail.len());
        out.push(&text[start..start + end]);
        cursor = start + end + usize::from(end < tail.len());
    }
    out
}
fn value<'a>(text: &'a str, tag: &str) -> Option<&'a str> {
    let upper = text.to_ascii_uppercase();
    let needle = format!("<{}", tag.to_ascii_uppercase());
    let start0 = upper.find(&needle)?;
    let after_name = start0 + needle.len();
    let next = upper.as_bytes().get(after_name).copied()?;
    if next != b'>' && !next.is_ascii_whitespace() {
        return None;
    }
    let open_end = upper[after_name..].find('>')? + after_name + 1;
    let tail = &text[open_end..];
    let end = tail.find('<').unwrap_or(tail.len());
    Some(tail[..end].trim())
}
fn clean(v: &str) -> String {
    v.split_whitespace().collect::<Vec<_>>().join(" ")
}
fn parse_ofx_date(v: &str) -> Result<Date, String> {
    let digits: String = v.trim().chars().take_while(char::is_ascii_digit).collect();
    if digits.len() < 8 {
        return Err("expected YYYYMMDD date".into());
    }
    let year: i32 = digits[0..4].parse().map_err(|_| "invalid year")?;
    let month: u8 = digits[4..6].parse().map_err(|_| "invalid month")?;
    let day: u8 = digits[6..8].parse().map_err(|_| "invalid day")?;
    Date::from_calendar_date(
        year,
        Month::try_from(month).map_err(|_| "invalid month")?,
        day,
    )
    .map_err(|e| e.to_string())
}
fn optional_money(text: &str, container: &str, tag: &str) -> Result<Option<Money>, ImportError> {
    let Some(start) = text.to_ascii_uppercase().find(&format!("<{container}")) else {
        return Ok(None);
    };
    let section = &text[start..];
    value(section, tag)
        .map(|v| {
            Money::parse_import(v, ImportRounding::HalfAwayFromZero)
                .map_err(|e| field(container, tag, e.to_string()))
        })
        .transpose()
}
fn field(location: &str, name: &str, message: String) -> ImportError {
    ImportError::Field {
        location: location.into(),
        field: name.into(),
        message,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const SGML: &str = "OFXHEADER:100\nDATA:OFXSGML\nVERSION:102\nENCODING:USASCII\n\n<OFX><BANKMSGSRSV1><STMTTRNRS><STMTRS><CURDEF>USD<BANKACCTFROM><BANKID>1<ACCTID>42<ACCTTYPE>CHECKING</BANKACCTFROM><BANKTRANLIST><DTSTART>20260101<DTEND>20260131<STMTTRN><TRNTYPE>DEBIT<DTPOSTED>20260102120000.000[-5:EST]<TRNAMT>-12.34<FITID>x1<NAME> Coffee   Shop<MEMO>hello</STMTTRN></BANKTRANLIST><LEDGERBAL><BALAMT>99.01</LEDGERBAL></STMTRS></STMTTRNRS></BANKMSGSRSV1></OFX>\0\0";
    #[test]
    fn parses_sgml_timezone_and_missing_leaf_closers() {
        let s = parse(SGML.as_bytes()).unwrap();
        assert_eq!(s.transactions.len(), 1);
        assert_eq!(s.transactions[0].posted_date.to_string(), "2026-01-02");
        assert_eq!(s.transactions[0].amount.minor_units(), -1234);
        assert_eq!(s.transactions[0].payee.as_deref(), Some("Coffee Shop"));
    }
    #[test]
    fn reports_transaction_and_field() {
        let bad = SGML.replace("-12.34", "wrong");
        let error = parse(bad.as_bytes()).unwrap_err().to_string();
        assert!(error.contains("FITID x1") && error.contains("TRNAMT"));
    }
    #[test]
    fn content_detection_is_not_extension_driven() {
        assert_eq!(
            crate::importing::source::detect(
                SGML.as_bytes(),
                Some(std::path::Path::new("download.txt"))
            ),
            crate::importing::source::Detection::Certain(
                crate::importing::source::ImportFormat::Ofx
            )
        );
    }
}
