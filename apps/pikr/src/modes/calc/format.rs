//! Writing a result out for a locale: exactly where it can be shown
//! exactly, with its repeating block marked (`0.(3)`), and rounded to
//! [`OUTPUT_DIGITS`] significant digits where it cannot.

use num_bigint::BigInt;
use num_integer::Integer;
use num_rational::BigRational;
use num_traits::{Pow, Signed, ToPrimitive, Zero};

use super::number::Num;

/// Significant digits of a rounded result: an approximate one, and a
/// repeating one when it is printed or copied.
const OUTPUT_DIGITS: u32 = 20;

/// An approximate result that agrees with a decimal of at most
/// [`SHORT_DIGITS`] significant digits to this many digits is that decimal
/// (`sin(30°)` is 0.5, `sin(pi/2)` is 1): calc's precision carries far more
/// digits than this, so agreement that long is not a coincidence.
const SNAP_DIGITS: u32 = 40;

/// See [`SNAP_DIGITS`].
const SHORT_DIGITS: usize = 15;

/// An approximate result smaller than 10^-this is zero: what is left of a
/// cancellation (`sin(pi)`) at calc's precision, not a value.
const NOISE_EXPONENT: u32 = 45;

/// The most fractional digits a repeating expansion is shown with, block
/// included (`1/17` repeats every 16 and fits; `5000/1609.344`, km in miles,
/// repeats every 42 and is shown rounded instead).
const MAX_EXPANSION: usize = 20;

/// Whole numbers with more digits than this are shown in scientific
/// notation in the row; they still print in full.
const MAX_SHOWN_DIGITS: usize = 30;

/// Whole numbers with more digits than this print in scientific notation.
const MAX_OUTPUT_DIGITS: usize = 1000;

/// A number whose first digit is at 10^this or above is written in
/// scientific notation, as is one whose first digit is below
/// 10^[`SCIENTIFIC_BELOW`].
const SCIENTIFIC_ABOVE: i64 = 21;

/// See [`SCIENTIFIC_ABOVE`].
const SCIENTIFIC_BELOW: i64 = -7;

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

    /// Rewrites a canonical number (`-1234.5`, `0.1(6)`, `1.5e-8`) for this
    /// locale, grouping its whole part when asked.
    fn localize(self, canonical: &str, grouped: bool) -> String {
        let (sign, rest) = canonical
            .strip_prefix('-')
            .map_or(("", canonical), |r| ("-", r));
        let (mantissa, exponent) = rest.find('e').map_or((rest, ""), |i| rest.split_at(i));
        let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
        let mut out = String::from(sign);
        for (i, digit) in whole.chars().enumerate() {
            if grouped && i > 0 && (whole.len() - i) % 3 == 0 {
                out.push(self.group);
            }
            out.push(digit);
        }
        if !fraction.is_empty() {
            out.push(self.decimal);
            out.push_str(fraction);
        }
        out.push_str(exponent);
        out
    }
}

/// A result as it is shown and as it is printed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rendered {
    /// For the row: grouped, a repeating block in parentheses.
    pub shown: String,
    /// For stdout and the clipboard: no grouping, rounded where the value
    /// does not end.
    pub output: String,
    /// Whether `shown` is not the exact value.
    pub approximate: bool,
}

/// `n` written for `seps`.
pub fn render(n: &Num, seps: Separators) -> Option<Rendered> {
    match n {
        Num::Exact(r) => Some(render_exact(r, seps)),
        Num::Approx(_) => {
            let r = n.to_rational()?;
            if r.abs() < power_of_ten(-i64::from(NOISE_EXPONENT)) {
                return Some(render_exact(&BigRational::zero(), seps));
            }
            let (negative, digits, k) = significant(&r, SNAP_DIGITS);
            if digits.len() <= SHORT_DIGITS {
                return Some(render_exact(&from_digits(negative, &digits, k), seps));
            }
            Some(rounded(&r, seps, true))
        }
    }
}

/// `n` rounded half away from zero to `decimals` places and always written
/// with that many (`92.50`), as money is. Marked approximate: exchange rates
/// are.
pub fn render_fixed(n: &Num, decimals: u32, seps: Separators) -> Option<Rendered> {
    let units = (n.to_rational()? * power_of_ten(i64::from(decimals)))
        .round()
        .to_integer();
    let places = decimals as usize;
    let digits = format!("{:0>width$}", units.abs(), width = places + 1);
    let (whole, fraction) = digits.split_at(digits.len() - places);
    let sign = if units.is_negative() { "-" } else { "" };
    let point = if places == 0 { "" } else { "." };
    let text = format!("{sign}{whole}{point}{fraction}");
    Some(Rendered {
        shown: seps.localize(&text, true),
        output: seps.localize(&text, false),
        approximate: true,
    })
}

fn render_exact(r: &BigRational, seps: Separators) -> Rendered {
    let plain = |text: &str| Rendered {
        shown: seps.localize(text, true),
        output: seps.localize(text, false),
        approximate: false,
    };
    if r.is_integer() {
        let text = r.to_integer().to_string();
        let digits = text.trim_start_matches('-').len();
        if digits <= MAX_SHOWN_DIGITS {
            return plain(&text);
        }
        let mut out = rounded(r, seps, true);
        if digits <= MAX_OUTPUT_DIGITS {
            out.output = text;
        }
        return out;
    }
    let (negative, digits, k) = significant(r, OUTPUT_DIGITS);
    let ends_within_output = from_digits(negative, &digits, k) == *r;
    if !(SCIENTIFIC_BELOW..SCIENTIFIC_ABOVE).contains(&k) {
        return rounded(r, seps, !ends_within_output);
    }
    match expansion(r, MAX_EXPANSION) {
        Some((whole, before, repeat)) => {
            let sign = if r.is_negative() { "-" } else { "" };
            let fixed = format!("{sign}{whole}.{before}");
            if repeat.is_empty() {
                return plain(&fixed);
            }
            let mut out = rounded(r, seps, false);
            out.shown = seps.localize(&format!("{fixed}({repeat})"), true);
            out
        }
        None => rounded(r, seps, true),
    }
}

/// `r` rounded to [`OUTPUT_DIGITS`] significant digits, both ways.
fn rounded(r: &BigRational, seps: Separators, approximate: bool) -> Rendered {
    let (negative, digits, k) = significant(r, OUTPUT_DIGITS);
    let text = layout(negative, &digits, k);
    Rendered {
        shown: seps.localize(&text, true),
        output: seps.localize(&text, false),
        approximate,
    }
}

/// Canonical text (`.` decimal, no grouping) for the digits `digits` whose
/// first digit is at 10^`k`.
fn layout(negative: bool, digits: &str, k: i64) -> String {
    let sign = if negative { "-" } else { "" };
    if !(SCIENTIFIC_BELOW..SCIENTIFIC_ABOVE).contains(&k) {
        let (first, rest) = digits.split_at(1);
        let point = if rest.is_empty() { "" } else { "." };
        return format!("{sign}{first}{point}{rest}e{k}");
    }
    let whole_len = usize::try_from(k + 1).unwrap_or(0);
    if whole_len == 0 {
        let zeros = "0".repeat(usize::try_from(-k - 1).unwrap_or(0));
        return format!("{sign}0.{zeros}{digits}");
    }
    if digits.len() <= whole_len {
        let zeros = "0".repeat(whole_len - digits.len());
        return format!("{sign}{digits}{zeros}");
    }
    let (whole, fraction) = digits.split_at(whole_len);
    format!("{sign}{whole}.{fraction}")
}

/// The value of [`significant`]'s answer.
fn from_digits(negative: bool, digits: &str, k: i64) -> BigRational {
    let mantissa: BigInt = digits.parse().unwrap_or_default();
    let len = i64::try_from(digits.len()).unwrap_or(0);
    let value = BigRational::from_integer(mantissa) * power_of_ten(k + 1 - len);
    if negative { -value } else { value }
}

fn power_of_ten(k: i64) -> BigRational {
    let p = BigRational::from_integer(Pow::pow(&BigInt::from(10), k.unsigned_abs()));
    if k >= 0 { p } else { p.recip() }
}

/// `r` rounded half away from zero to `digits` significant digits: its
/// sign, the digits with trailing zeros dropped, and the power of ten of the
/// first digit (`1234.5` to 3 digits is `(false, "123", 3)`).
fn significant(r: &BigRational, digits: u32) -> (bool, String, i64) {
    if r.is_zero() {
        return (false, "0".to_owned(), 0);
    }
    let magnitude = r.abs();
    let len = |n: &BigInt| i64::try_from(n.to_string().len()).unwrap_or(0);
    // An estimate within one of the first digit's power, then corrected.
    let mut k = len(magnitude.numer()) - len(magnitude.denom());
    while magnitude < power_of_ten(k) {
        k -= 1;
    }
    while magnitude >= power_of_ten(k + 1) {
        k += 1;
    }
    let scaled = &magnitude * power_of_ten(i64::from(digits) - 1 - k);
    let mut m = scaled.round().to_integer();
    if m.to_string().len() > digits as usize {
        m /= 10;
        k += 1;
    }
    let text = m.to_string();
    let text = text.trim_end_matches('0');
    (r.is_negative(), text.to_owned(), k)
}

/// The decimal expansion of `|r|`: its whole part, the fractional digits
/// before any repetition, and the repeating block (`1/6` is `0`, `1`, `6`).
/// `None` when it neither ends nor repeats within `limit` digits.
fn expansion(r: &BigRational, limit: usize) -> Option<(BigInt, String, String)> {
    let magnitude = r.abs();
    let denom = magnitude.denom();
    let (whole, mut remainder) = magnitude.numer().div_rem(denom);
    let mut digits = String::new();
    let mut seen: Vec<BigInt> = Vec::new();
    while !remainder.is_zero() {
        if let Some(start) = seen.iter().position(|s| *s == remainder) {
            let repeat = digits.split_off(start);
            return Some((whole, digits, repeat));
        }
        if digits.len() == limit {
            return None;
        }
        seen.push(remainder.clone());
        let (digit, rest): (BigInt, BigInt) = (remainder * BigInt::from(10)).div_rem(denom);
        digits.push(char::from(b'0' + digit.to_u8()?));
        remainder = rest;
    }
    Some((whole, digits, String::new()))
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

    fn r(n: i64, d: i64) -> BigRational {
        BigRational::new(n.into(), d.into())
    }

    fn show(n: i64, d: i64) -> (String, String) {
        let out = render(&Num::Exact(r(n, d)), EN).unwrap();
        (out.shown, out.output)
    }

    #[test]
    fn exact_values_print_exactly() {
        assert_eq!(show(3, 1), ("3".into(), "3".into()));
        assert_eq!(show(5, 2), ("2.5".into(), "2.5".into()));
        assert_eq!(
            show(-2_469_135, 2),
            ("-1,234,567.5".into(), "-1234567.5".into())
        );
        assert_eq!(show(3, 10), ("0.3".into(), "0.3".into()));
    }

    #[test]
    fn repeating_values_mark_the_block_and_print_twenty_digits() {
        assert_eq!(
            show(1, 3),
            ("0.(3)".into(), "0.33333333333333333333".into())
        );
        assert_eq!(
            show(2, 3),
            ("0.(6)".into(), "0.66666666666666666667".into())
        );
        assert_eq!(
            show(1, 6),
            ("0.1(6)".into(), "0.16666666666666666667".into())
        );
        assert_eq!(
            show(22, 7),
            ("3.(142857)".into(), "3.1428571428571428571".into())
        );
        assert_eq!(
            show(-4000, 3),
            ("-1,333.(3)".into(), "-1333.3333333333333333".into())
        );
        // 96 repeating digits: rounded instead, and marked approximate.
        assert_eq!(show(1, 17).0, "0.(0588235294117647)");
        let out = render(&Num::Exact(r(1, 97)), EN).unwrap();
        assert!(out.approximate);
        assert_eq!(out.output, "0.010309278350515463918");
    }

    #[test]
    fn extremes_use_scientific_notation() {
        assert_eq!(show(1, 1_000_000_000), ("1e-9".into(), "1e-9".into()));
        let tiny = render(&Num::Exact(r(1, 3_000_000_000)), EN).unwrap();
        assert_eq!(tiny.output, "3.3333333333333333333e-10");
        assert!(tiny.approximate);
        let fifty = (1..=50).fold(BigInt::from(1), |a, k| a * k);
        let out = render(&Num::Exact(BigRational::from_integer(fifty.clone())), EN).unwrap();
        // 50! is 30414093201713378043|6…: the 21st digit rounds the 20th up.
        assert_eq!(out.shown, "3.0414093201713378044e64");
        assert_eq!(out.output, fifty.to_string());
    }

    #[test]
    fn approximate_values_round_and_snap() {
        let root2 = render(&Num::int(2).root(2).unwrap(), EN).unwrap();
        assert_eq!(root2.output, "1.4142135623730950488");
        assert!(root2.approximate);
        // pi/2 through sin is 1 to far more than 40 digits.
        let half_pi = Num::pi().unwrap().div(&Num::int(2)).unwrap();
        let one = render(&half_pi.apply(astro_float::BigFloat::sin).unwrap(), EN).unwrap();
        assert_eq!((one.output.as_str(), one.approximate), ("1", false));
        let zero = render(
            &Num::pi()
                .unwrap()
                .apply(astro_float::BigFloat::sin)
                .unwrap(),
            EN,
        )
        .unwrap();
        assert_eq!(zero.output, "0");
    }

    #[test]
    fn money_keeps_its_decimals() {
        let fixed = |n: i64, d: i64, places| {
            render_fixed(&Num::Exact(r(n, d)), places, EN).map(|out| out.shown)
        };
        assert_eq!(fixed(185, 2, 2).as_deref(), Some("92.50"));
        assert_eq!(fixed(-1, 3, 2).as_deref(), Some("-0.33"));
        assert_eq!(fixed(1, 1000, 2).as_deref(), Some("0.00"));
        assert_eq!(fixed(1_234_567, 2, 0).as_deref(), Some("617,284"));
        assert_eq!(fixed(1_234_567, 1, 2).as_deref(), Some("1,234,567.00"));
    }

    #[test]
    fn the_locale_writes_both_forms() {
        let out = render(&Num::Exact(r(-2_469_135, 2)), DE).unwrap();
        assert_eq!(
            (out.shown.as_str(), out.output.as_str()),
            ("-1.234.567,5", "-1234567,5")
        );
        let third = render(&Num::Exact(r(1, 3)), DE).unwrap();
        assert_eq!(third.shown, "0,(3)");
        assert_eq!(DE.argument(), ';');
        assert_eq!(EN.argument(), ',');
    }
}
