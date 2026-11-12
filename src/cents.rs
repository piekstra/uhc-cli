//! Amounts as whole cents. A statement prints `$1,234.56`; arithmetic
//! (line sums, remaining balances) stays in integers, and output goes out as
//! `pk_cli_core::Money`'s string decimal, never a float.

use pk_cli_core::Money;

/// An amount in US cents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Cents(pub i64);

impl Cents {
    /// Parse `$1,234.56`, `1234.56`, `-$5.00` or `($5.00)`. Exactly two
    /// fraction digits, as a statement prints them; anything else is `None`.
    pub fn parse(raw: &str) -> Option<Cents> {
        let s = raw.trim();
        let (negative, s) = if let Some(inner) = s.strip_prefix('(').and_then(|r| r.strip_suffix(')')) {
            (true, inner.trim())
        } else if let Some(rest) = s.strip_prefix('-') {
            (true, rest.trim())
        } else {
            (false, s)
        };
        let s = s.strip_prefix('$').unwrap_or(s);
        let (whole, frac) = s.split_once('.')?;
        if frac.len() != 2 || !frac.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        if whole.is_empty() || !whole.bytes().all(|b| b.is_ascii_digit() || b == b',') {
            return None;
        }
        // Thousands separators must group by three.
        let groups: Vec<&str> = whole.split(',').collect();
        if groups.len() > 1 && (groups[0].is_empty() || groups[0].len() > 3 || groups[1..].iter().any(|g| g.len() != 3))
        {
            return None;
        }
        let digits: String = whole.chars().filter(|c| *c != ',').collect();
        let units: i64 = digits.parse().ok()?;
        let cents = units.checked_mul(100)?.checked_add(frac.parse::<i64>().ok()?)?;
        Some(Cents(if negative { -cents } else { cents }))
    }

    pub fn money(self) -> Money {
        Money::from_cents(self.0)
    }

    /// The FHIR `Money.value` (a JSON number with two decimals' worth of
    /// precision; the shortest representation round-trips exactly).
    pub fn fhir_value(self) -> serde_json::Value {
        let n = serde_json::Number::from_f64(self.0 as f64 / 100.0).expect("finite");
        serde_json::Value::Number(n)
    }
}

impl std::ops::Add for Cents {
    type Output = Cents;
    fn add(self, rhs: Cents) -> Cents {
        // Statement amounts are far from the limit; a corrupt one saturates
        // rather than wrapping or panicking.
        Cents(self.0.saturating_add(rhs.0))
    }
}

impl std::iter::Sum for Cents {
    fn sum<I: Iterator<Item = Cents>>(iter: I) -> Cents {
        iter.fold(Cents(0), |a, b| a + b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statement_amounts_parse_to_cents() {
        assert_eq!(Cents::parse("$161.98"), Some(Cents(16_198)));
        assert_eq!(Cents::parse("$13,000.00"), Some(Cents(1_300_000)));
        assert_eq!(Cents::parse("$0.00"), Some(Cents(0)));
        assert_eq!(Cents::parse("($5.25)"), Some(Cents(-525)));
        assert_eq!(Cents::parse("-$5.25"), Some(Cents(-525)));
        assert_eq!(Cents::parse("1,234,567.89"), Some(Cents(123_456_789)));
    }

    #[test]
    fn anything_but_two_decimals_is_refused() {
        for bad in ["$5", "$5.1", "$5.123", "", "$", "abc", "$1,23.00", "$,123.00", "Met"] {
            assert_eq!(Cents::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn fhir_values_keep_two_decimals_exactly() {
        assert_eq!(Cents(7713).fhir_value().to_string(), "77.13");
        assert_eq!(Cents(100_000).fhir_value().to_string(), "1000.0");
        assert_eq!(Cents(-525).money().amount, "-5.25");
    }
}
