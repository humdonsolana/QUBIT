use anyhow::{anyhow, bail, Result};

/// Parses a decimal amount such as `1.5` into base units with `decimals` fractional digits.
pub fn parse_amount(text: &str, decimals: u8) -> Result<u64> {
    let (whole, fraction) = text.split_once('.').unwrap_or((text, ""));
    if whole.is_empty() && fraction.is_empty() {
        bail!("empty amount");
    }
    if !whole
        .bytes()
        .chain(fraction.bytes())
        .all(|b| b.is_ascii_digit())
    {
        bail!("amount must be a decimal number: {text}");
    }
    if fraction.len() > usize::from(decimals) {
        bail!("{text} has more than {decimals} decimal places");
    }
    format!("{whole}{fraction:0<width$}", width = usize::from(decimals))
        .parse()
        .map_err(|_| anyhow!("amount too large"))
}

/// Formats base units with `decimals` fractional digits, trimming trailing zeros.
pub fn format_amount(raw: u64, decimals: u8) -> String {
    let decimals = usize::from(decimals);
    let padded = format!("{raw:0>width$}", width = decimals + 1);
    let (whole, fraction) = padded.split_at(padded.len() - decimals);
    let fraction = fraction.trim_end_matches('0');
    if fraction.is_empty() {
        whole.to_owned()
    } else {
        format!("{whole}.{fraction}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sol_amounts() {
        assert_eq!(parse_amount("1", 9).unwrap(), 1_000_000_000);
        assert_eq!(parse_amount("1.5", 9).unwrap(), 1_500_000_000);
        assert_eq!(parse_amount("0.000000001", 9).unwrap(), 1);
        assert_eq!(parse_amount(".5", 9).unwrap(), 500_000_000);
        assert_eq!(parse_amount("250", 6).unwrap(), 250_000_000);
        assert!(parse_amount("1.0000000001", 9).is_err());
        assert!(parse_amount("abc", 9).is_err());
        assert!(parse_amount("", 9).is_err());
        assert!(parse_amount("99999999999999999999", 9).is_err());
    }

    #[test]
    fn formats_amounts() {
        assert_eq!(format_amount(1_500_000_000, 9), "1.5");
        assert_eq!(format_amount(1_000_000_000, 9), "1");
        assert_eq!(format_amount(1, 9), "0.000000001");
        assert_eq!(format_amount(42, 0), "42");
        assert_eq!(format_amount(0, 6), "0");
    }

    #[test]
    fn formats_amounts_beyond_u128_scale() {
        assert_eq!(
            format_amount(u64::MAX, 255),
            format!("0.{}18446744073709551615", "0".repeat(235))
        );
    }
}
