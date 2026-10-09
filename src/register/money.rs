//! Decimal input and formatting without floating-point conversion.

/// Parses nonnegative CAD input, allowing at most two decimal places.
pub fn parse_amount(input: &str) -> Result<i64, &'static str> {
    let input = input.trim();
    let mut parts = input.split('.');
    let whole = parts.next().unwrap_or_default();
    let fraction = parts.next().unwrap_or_default();
    if whole.is_empty()
        || !whole.bytes().all(|byte| byte.is_ascii_digit())
        || fraction.len() > 2
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
        || parts.next().is_some()
    {
        return Err("Enter an amount such as 25.00, with at most two decimal places.");
    }
    let dollars: i64 = whole.parse().map_err(|_| "The amount is too large.")?;
    let cents = match fraction.len() {
        0 => 0,
        1 => i64::from(fraction.as_bytes()[0] - b'0') * 10,
        _ => {
            i64::from(fraction.as_bytes()[0] - b'0') * 10 + i64::from(fraction.as_bytes()[1] - b'0')
        }
    };
    dollars
        .checked_mul(100)
        .and_then(|amount| amount.checked_add(cents))
        .ok_or("The amount is too large.")
}

/// Formats integer minor units with grouping and exactly two decimal places.
pub fn format_amount(amount: i64) -> String {
    let absolute = amount.unsigned_abs();
    let digits = (absolute / 100).to_string();
    let mut grouped = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    format!(
        "{}{}.{:02}",
        if amount < 0 { "−" } else { "" },
        grouped,
        absolute % 100
    )
}

/// Formats a CAD amount, placing a negative sign before the currency symbol.
pub fn format_cad(amount: i64) -> String {
    let text = format_amount(amount);
    match text.strip_prefix('−') {
        Some(magnitude) => format!("−${magnitude}"),
        None => format!("${text}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_exact_decimal_amounts() {
        for (input, expected) in [
            ("0", 0),
            ("0.01", 1),
            (" 12.3 ", 1230),
            ("100.00", 10000),
            ("92233720368547758.07", i64::MAX),
        ] {
            assert_eq!(parse_amount(input), Ok(expected));
        }
    }
    #[test]
    fn rejects_invalid_and_overflowing_amounts() {
        for input in [
            "",
            "-1",
            "+1",
            "NaN",
            "1e3",
            "1,000",
            ".50",
            "1.234",
            "1.2.3",
            "92233720368547758.08",
        ] {
            assert!(parse_amount(input).is_err(), "{input}");
        }
    }
    #[test]
    fn formats_grouping_and_minimum_integer() {
        assert_eq!(format_amount(123456), "1,234.56");
        assert_eq!(format_amount(-1), "−0.01");
        assert_eq!(format_cad(-1), "−$0.01");
        assert_eq!(format_amount(i64::MIN), "−92,233,720,368,547,758.08");
    }
}
