use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};
use thiserror::Error;

/// A USD amount stored exclusively as signed minor units (cents).
#[derive(
    Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct Money(i64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImportRounding {
    /// Round to nearest cent; an exact tie rounds away from zero.
    HalfAwayFromZero,
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum MoneyError {
    #[error("invalid money text")]
    Invalid,
    #[error("money value is outside the supported range")]
    Overflow,
    #[error("more than two fractional digits require an explicit import rounding policy")]
    RoundingRequired,
}

impl Money {
    pub const ZERO: Self = Self(0);
    #[must_use]
    pub const fn from_minor_units(value: i64) -> Self {
        Self(value)
    }
    #[must_use]
    pub const fn minor_units(self) -> i64 {
        self.0
    }
    pub fn checked_add(self, rhs: Self) -> Result<Self, MoneyError> {
        self.0
            .checked_add(rhs.0)
            .map(Self)
            .ok_or(MoneyError::Overflow)
    }
    pub fn checked_sub(self, rhs: Self) -> Result<Self, MoneyError> {
        self.0
            .checked_sub(rhs.0)
            .map(Self)
            .ok_or(MoneyError::Overflow)
    }
    pub fn checked_neg(self) -> Result<Self, MoneyError> {
        self.0.checked_neg().map(Self).ok_or(MoneyError::Overflow)
    }
    pub fn checked_mul(self, rhs: i64) -> Result<Self, MoneyError> {
        self.0
            .checked_mul(rhs)
            .map(Self)
            .ok_or(MoneyError::Overflow)
    }
    pub fn parse_import(text: &str, rounding: ImportRounding) -> Result<Self, MoneyError> {
        parse(text, Some(rounding))
    }
}

impl FromStr for Money {
    type Err = MoneyError;
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        parse(text, None)
    }
}

fn parse(text: &str, rounding: Option<ImportRounding>) -> Result<Money, MoneyError> {
    let mut s = text.trim();
    let accounting = s.starts_with('(') && s.ends_with(')');
    if accounting {
        s = s[1..s.len() - 1].trim();
    }
    let explicit_negative = s.starts_with('-');
    if explicit_negative {
        s = &s[1..];
    }
    if s.starts_with('+') {
        s = &s[1..];
    }
    if let Some(rest) = s.strip_prefix('$') {
        s = rest.trim_start();
    }
    if s.is_empty() || (accounting && explicit_negative) {
        return Err(MoneyError::Invalid);
    }
    let (whole, fraction) = s.split_once('.').map_or((s, ""), |v| v);
    if fraction.contains('.') || !fraction.chars().all(|c| c.is_ascii_digit()) {
        return Err(MoneyError::Invalid);
    }
    let digits = if whole.contains(',') {
        let groups: Vec<_> = whole.split(',').collect();
        if groups.is_empty()
            || groups[0].is_empty()
            || groups[0].len() > 3
            || groups.iter().skip(1).any(|g| g.len() != 3)
        {
            return Err(MoneyError::Invalid);
        }
        groups.concat()
    } else {
        whole.to_owned()
    };
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
        return Err(MoneyError::Invalid);
    }
    let whole: i128 = digits.parse().map_err(|_| MoneyError::Overflow)?;
    let first = fraction
        .as_bytes()
        .first()
        .map_or(0, |v| i128::from(v - b'0'));
    let second = fraction
        .as_bytes()
        .get(1)
        .map_or(0, |v| i128::from(v - b'0'));
    let mut cents = whole
        .checked_mul(100)
        .and_then(|v| v.checked_add(first * 10 + second))
        .ok_or(MoneyError::Overflow)?;
    if fraction.len() > 2 {
        rounding.ok_or(MoneyError::RoundingRequired)?;
        if fraction.as_bytes()[2] >= b'5' {
            cents = cents.checked_add(1).ok_or(MoneyError::Overflow)?;
        }
    }
    if accounting || explicit_negative {
        cents = -cents;
    }
    i64::try_from(cents)
        .map(Money)
        .map_err(|_| MoneyError::Overflow)
}

impl fmt::Display for Money {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let negative = self.0 < 0;
        let absolute = i128::from(self.0).abs();
        let whole = (absolute / 100).to_string();
        let mut grouped = String::new();
        for (i, ch) in whole.chars().enumerate() {
            if i > 0 && (whole.len() - i) % 3 == 0 {
                grouped.push(',');
            }
            grouped.push(ch);
        }
        write!(
            f,
            "{}${}.{:02}",
            if negative { "-" } else { "" },
            grouped,
            absolute % 100
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    #[test]
    fn parsing_and_formatting() {
        for (text, cents) in [
            ("$0", 0),
            (" 1,234.5 ", 123450),
            ("($12.34)", -1234),
            ("-$0.01", -1),
        ] {
            let m: Money = text.parse().unwrap();
            assert_eq!(m.minor_units(), cents);
            assert_eq!(m.to_string().parse(), Ok(m));
        }
        assert_eq!(
            Money::from_minor_units(i64::MIN).to_string().parse(),
            Ok(Money::from_minor_units(i64::MIN))
        );
        assert_eq!(
            Money::from_minor_units(i64::MAX).to_string().parse(),
            Ok(Money::from_minor_units(i64::MAX))
        );
        for bad in ["1,00", "12,34.00", "1 2", "１２", "$", "1.234"] {
            assert!(bad.parse::<Money>().is_err(), "{bad}");
        }
        assert_eq!(
            Money::parse_import("1.235", ImportRounding::HalfAwayFromZero)
                .unwrap()
                .minor_units(),
            124
        );
        assert_eq!(
            Money::parse_import("-1.235", ImportRounding::HalfAwayFromZero)
                .unwrap()
                .minor_units(),
            -124
        );
    }
    #[test]
    fn checked_arithmetic() {
        assert!(
            Money::from_minor_units(i64::MAX)
                .checked_add(Money::from_minor_units(1))
                .is_err()
        );
        assert!(Money::from_minor_units(i64::MIN).checked_neg().is_err());
    }

    proptest! {
        #[test]
        fn display_parse_round_trip(value in any::<i64>()) {
            let money = Money::from_minor_units(value);
            prop_assert_eq!(money.to_string().parse::<Money>(), Ok(money));
        }

        #[test]
        fn checked_add_matches_wide_math(left in any::<i64>(), right in any::<i64>()) {
            let wide = i128::from(left) + i128::from(right);
            let actual = Money::from_minor_units(left).checked_add(Money::from_minor_units(right));
            if let Ok(expected) = i64::try_from(wide) {
                prop_assert_eq!(actual.unwrap().minor_units(), expected);
            } else {
                prop_assert_eq!(actual, Err(MoneyError::Overflow));
            }
        }
    }
}
