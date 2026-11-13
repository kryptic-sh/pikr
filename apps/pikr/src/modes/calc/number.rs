//! Calc's numbers: exact integers while they fit, floats otherwise, and how
//! a result is written out for a locale.

/// A value during evaluation. Integer arithmetic stays exact while it fits
/// an `i128` and falls back to a float when it would overflow; a float that
/// is not finite (`1.0/0`, `sqrt(-1)`) is no result at all.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Num {
    Int(i128),
    Float(f64),
}

/// The largest `n` whose factorial is still a finite `f64` (171! overflows).
const MAX_FLOAT_FACTORIAL: i128 = 170;

/// Significant digits a float result is rounded to before it is shown, so
/// binary noise (`0.1 + 0.2` = `0.30000000000000004`) prints as `0.3`. An
/// `f64` carries 15–17; 15 is the most that always round-trip.
const SIGNIFICANT_DIGITS: usize = 15;

/// Floats at or above this magnitude are written in scientific notation.
const SCIENTIFIC_ABOVE: f64 = 1e21;

/// Non-zero floats below this magnitude are written in scientific notation.
const SCIENTIFIC_BELOW: f64 = 1e-7;

impl Num {
    pub fn to_f64(self) -> f64 {
        match self {
            // Precision loss past 2^53 is the point of falling back to float.
            #[allow(clippy::cast_precision_loss)]
            Self::Int(i) => i as f64,
            Self::Float(f) => f,
        }
    }

    /// `Some` only when finite: every operation ends here, so an infinite or
    /// NaN result never reaches the user.
    pub fn float(f: f64) -> Option<Self> {
        f.is_finite().then_some(Self::Float(f))
    }

    /// A float that holds a whole number small enough to be exact, as an
    /// integer; anything else unchanged.
    fn integral(self) -> Option<i128> {
        match self {
            Self::Int(i) => Some(i),
            // 2^53: the last float magnitude where every integer is exact.
            Self::Float(f) if f.fract() == 0.0 && f.abs() <= 9_007_199_254_740_992.0 => {
                // The guard bounds `f` well inside `i128`.
                #[allow(clippy::cast_possible_truncation)]
                Some(f as i128)
            }
            Self::Float(_) => None,
        }
    }

    pub fn add(self, rhs: Self) -> Option<Self> {
        self.int_op(rhs, i128::checked_add)
            .map_or_else(|| Self::float(self.to_f64() + rhs.to_f64()), Some)
    }

    pub fn sub(self, rhs: Self) -> Option<Self> {
        self.int_op(rhs, i128::checked_sub)
            .map_or_else(|| Self::float(self.to_f64() - rhs.to_f64()), Some)
    }

    pub fn mul(self, rhs: Self) -> Option<Self> {
        self.int_op(rhs, i128::checked_mul)
            .map_or_else(|| Self::float(self.to_f64() * rhs.to_f64()), Some)
    }

    /// Division is exact when it divides evenly, a float otherwise; dividing
    /// by zero is no result, never infinity.
    pub fn div(self, rhs: Self) -> Option<Self> {
        if rhs.to_f64() == 0.0 {
            return None;
        }
        if let (Self::Int(a), Self::Int(b)) = (self, rhs)
            && let Some(0) = a.checked_rem(b)
            && let Some(q) = a.checked_div(b)
        {
            return Some(Self::Int(q));
        }
        Self::float(self.to_f64() / rhs.to_f64())
    }

    /// The remainder, with the sign of the dividend (Rust's `%`, and most
    /// calculators' `mod`).
    pub fn rem(self, rhs: Self) -> Option<Self> {
        if rhs.to_f64() == 0.0 {
            return None;
        }
        self.int_op(rhs, i128::checked_rem)
            .map_or_else(|| Self::float(self.to_f64() % rhs.to_f64()), Some)
    }

    pub fn pow(self, rhs: Self) -> Option<Self> {
        if let (Self::Int(base), Self::Int(exp)) = (self, rhs)
            && let Ok(exp) = u32::try_from(exp)
            && let Some(p) = base.checked_pow(exp)
        {
            return Some(Self::Int(p));
        }
        Self::float(self.to_f64().powf(rhs.to_f64()))
    }

    pub fn neg(self) -> Option<Self> {
        match self {
            Self::Int(i) => i
                .checked_neg()
                .map(Self::Int)
                .or_else(|| Self::float(-self.to_f64())),
            Self::Float(f) => Self::float(-f),
        }
    }

    /// `n!` for a whole `n ≥ 0`: exact while it fits, then a float up to the
    /// largest finite one.
    pub fn factorial(self) -> Option<Self> {
        let n = self.integral()?;
        if !(0..=MAX_FLOAT_FACTORIAL).contains(&n) {
            return None;
        }
        let mut exact: Option<i128> = Some(1);
        let mut approx = 1.0_f64;
        for k in 2..=n {
            exact = exact.and_then(|e| e.checked_mul(k));
            #[allow(clippy::cast_precision_loss)]
            {
                approx *= k as f64;
            }
        }
        exact.map_or_else(|| Self::float(approx), |e| Some(Self::Int(e)))
    }

    /// A float rounding function's result, as an integer when it is one.
    pub fn whole(f: f64) -> Option<Self> {
        Self::float(f).map(|n| n.integral().map_or(n, Self::Int))
    }

    fn int_op(self, rhs: Self, op: fn(i128, i128) -> Option<i128>) -> Option<Self> {
        match (self, rhs) {
            (Self::Int(a), Self::Int(b)) => op(a, b).map(Self::Int),
            _ => None,
        }
    }
}

/// The characters a locale writes numbers with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Separators {
    /// Between the whole and the fractional part: `.` in `1.5`, `,` in `1,5`.
    pub decimal: char,
    /// Between groups of three digits: `,` in `1,000`, `.` in `1.000`.
    pub group: char,
}

impl Default for Separators {
    fn default() -> Self {
        Self {
            decimal: '.',
            group: ',',
        }
    }
}

impl Separators {
    /// Between a function's arguments: `,`, or `;` where `,` is the decimal
    /// mark (as spreadsheets do). `;` is accepted everywhere.
    pub fn argument(self) -> char {
        if self.decimal == ',' { ';' } else { ',' }
    }

    /// `n` the way the row shows it: the locale's decimal mark, and its
    /// whole part grouped by threes.
    pub fn shown(self, n: Num) -> String {
        self.localize(&canonical(n), true)
    }

    /// `n` the way it is printed and copied: the locale's decimal mark and no
    /// grouping, so it pastes as a number into other programs.
    pub fn plain(self, n: Num) -> String {
        self.localize(&canonical(n), false)
    }

    /// Rewrites a canonical number (`-1234.5`, `1.5e-8`) for this locale.
    fn localize(self, canonical: &str, grouped: bool) -> String {
        let (sign, rest) = canonical
            .strip_prefix('-')
            .map_or(("", canonical), |r| ("-", r));
        let (mantissa, exponent) = rest.find('e').map_or((rest, ""), |i| rest.split_at(i));
        let (whole, fraction) = mantissa
            .find('.')
            .map_or((mantissa, ""), |i| (&mantissa[..i], &mantissa[i + 1..]));
        let mut out = String::from(sign);
        if grouped {
            for (i, digit) in whole.chars().enumerate() {
                if i > 0 && (whole.len() - i) % 3 == 0 {
                    out.push(self.group);
                }
                out.push(digit);
            }
        } else {
            out.push_str(whole);
        }
        if !fraction.is_empty() {
            out.push(self.decimal);
            out.push_str(fraction);
        }
        out.push_str(exponent);
        out
    }
}

/// `n` in Rust's own notation (`.` decimal, no grouping), floats rounded to
/// [`SIGNIFICANT_DIGITS`] and switched to scientific notation outside the
/// range a row can show in full.
fn canonical(n: Num) -> String {
    match n {
        Num::Int(i) => i.to_string(),
        Num::Float(f) => {
            // Printing in scientific notation with a fixed digit count rounds
            // to that many significant digits; parsing it back gives the float
            // nearest the rounded value, which Rust then prints shortest.
            let rounded: f64 = format!("{f:.*e}", SIGNIFICANT_DIGITS - 1)
                .parse()
                .unwrap_or(f);
            let magnitude = rounded.abs();
            if magnitude == 0.0 {
                "0".to_owned()
            } else if !(SCIENTIFIC_BELOW..SCIENTIFIC_ABOVE).contains(&magnitude) {
                format!("{rounded:e}")
            } else {
                format!("{rounded}")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EN: Separators = Separators {
        decimal: '.',
        group: ',',
    };
    const DE: Separators = Separators {
        decimal: ',',
        group: '.',
    };

    #[test]
    fn integer_overflow_falls_back_to_float() {
        let big = Num::Int(9_999_999_999);
        assert_eq!(big.mul(big), Some(Num::Int(99_999_999_980_000_000_001)));
        let huge = Num::Int(i128::MAX);
        assert!(matches!(huge.add(Num::Int(1)), Some(Num::Float(_))));
    }

    #[test]
    fn division_is_exact_only_when_it_divides() {
        assert_eq!(Num::Int(6).div(Num::Int(2)), Some(Num::Int(3)));
        assert_eq!(Num::Int(7).div(Num::Int(2)), Some(Num::Float(3.5)));
        assert_eq!(Num::Int(1).div(Num::Int(0)), None);
        assert_eq!(Num::Float(1.0).div(Num::Float(0.0)), None);
    }

    #[test]
    fn factorial_is_exact_then_float_then_none() {
        assert_eq!(Num::Int(5).factorial(), Some(Num::Int(120)));
        assert_eq!(Num::Int(0).factorial(), Some(Num::Int(1)));
        assert!(matches!(Num::Int(40).factorial(), Some(Num::Float(_))));
        assert_eq!(Num::Int(171).factorial(), None);
        assert_eq!(Num::Int(-1).factorial(), None);
        assert_eq!(Num::Float(2.5).factorial(), None);
    }

    #[test]
    fn floats_drop_binary_noise() {
        let sum = Num::Float(0.1).add(Num::Float(0.2)).unwrap();
        assert_eq!(EN.plain(sum), "0.3");
        assert_eq!(EN.plain(Num::Float(1.0 / 3.0)), "0.333333333333333");
    }

    #[test]
    fn extremes_switch_to_scientific_notation() {
        assert_eq!(EN.plain(Num::Float(1e21)), "1e21");
        assert_eq!(EN.plain(Num::Float(1e20)), "100000000000000000000");
        assert_eq!(EN.plain(Num::Float(1.5e-8)), "1.5e-8");
        assert_eq!(EN.plain(Num::Float(0.000_001)), "0.000001");
        assert_eq!(EN.plain(Num::Float(-0.0)), "0");
    }

    #[test]
    fn shown_groups_and_plain_does_not() {
        let n = Num::Float(-1_234_567.5);
        assert_eq!(EN.shown(n), "-1,234,567.5");
        assert_eq!(EN.plain(n), "-1234567.5");
        assert_eq!(DE.shown(n), "-1.234.567,5");
        assert_eq!(DE.plain(n), "-1234567,5");
        assert_eq!(EN.shown(Num::Int(999)), "999");
        assert_eq!(EN.shown(Num::Int(1000)), "1,000");
        assert_eq!(DE.shown(Num::Float(2.5e-9)), "2,5e-9");
    }

    #[test]
    fn the_argument_separator_avoids_the_decimal_mark() {
        assert_eq!(EN.argument(), ',');
        assert_eq!(DE.argument(), ';');
    }
}
