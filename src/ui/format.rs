//! Shared, locale-stable presentation formatting for the desktop UI.
use crate::domain::Money;
use time::Date;

/// Formats USD from signed minor units using integer arithmetic (including `i64::MIN`).
#[must_use]
pub fn money(value: Money) -> String {
    let cents = i128::from(value.minor_units());
    let negative = cents < 0;
    let absolute = cents.abs();
    let digits = (absolute / 100).to_string();
    let mut whole = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            whole.push(',');
        }
        whole.push(digit);
    }
    format!(
        "{}${}.{:02}",
        if negative { "-" } else { "" },
        whole,
        absolute % 100
    )
}

#[must_use]
pub fn date(date: Date) -> String {
    format!(
        "{:02}/{:02}/{:04}",
        u8::from(date.month()),
        date.day(),
        date.year()
    )
}

/// Narrow register columns intentionally use a two-digit year; all other dates use [`date`].
#[must_use]
pub fn register_date(date: Date) -> String {
    format!(
        "{:02}/{:02}/{:02}",
        u8::from(date.month()),
        date.day(),
        date.year().rem_euclid(100)
    )
}

/// A right-aligned amount whose sign remains meaningful without color perception.
pub fn money_cell(ui: &mut egui::Ui, value: Money) -> egui::Response {
    let negative = value < Money::ZERO;
    let text = egui::RichText::new(money(value))
        .monospace()
        .color(if negative {
            ui.visuals().error_fg_color
        } else {
            ui.visuals().strong_text_color()
        });
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.label(text)
    })
    .inner
    .on_hover_text(if negative {
        "Negative amount"
    } else {
        "Zero or positive amount"
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn formats_all_integer_money_without_floating_point() {
        for (cents, expected) in [
            (0, "$0.00"),
            (120_437, "$1,204.37"),
            (-8_423, "-$84.23"),
            (9_223_372_036_854_775_807, "$92,233,720,368,547,758.07"),
            (i64::MIN, "-$92,233,720,368,547,758.08"),
        ] {
            assert_eq!(money(Money::from_minor_units(cents)), expected);
        }
    }
}
