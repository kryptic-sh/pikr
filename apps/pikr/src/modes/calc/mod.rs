//! calc mode — reactive expression evaluator.
//!
//! Unlike the other modes, `Calc::collect()` returns an empty list. Calc is
//! driven by `AppState::rerank`, which short-circuits when `cli_mode == Calc`
//! and calls [`eval`] against the live query, producing a single synthetic
//! `Entry` per evaluation.
//!
//! The evaluator is pikr's own ([`lexer`], [`parser`], [`number`],
//! [`format`]): numbers are read and written with the locale's separators,
//! arithmetic is exact (rationals of any size), and what cannot be exact (π,
//! √2, logarithms, trigonometry) is computed to about 57 digits and shown
//! rounded, marked as approximate. Values carry units ([`units`],
//! [`value`]), and `X to Y` converts between them.

mod format;
mod lexer;
mod locale;
mod number;
mod parser;
mod rates;
mod units;
mod value;

use std::collections::HashMap;

use super::{Entry, Mode};
use anyhow::Result;

pub use format::Separators;
pub use locale::{separators, validate as validate_separators};
pub use rates::{Rates, load as load_rates};

/// Hard cap on expression length, in characters. The parser's own nesting
/// limit is what keeps it off the end of the stack; this only bounds the
/// work a pasted wall of text costs on every keystroke. Expressions over the
/// cap are rejected the way a parse error is (no result row).
const MAX_EXPR_LEN: usize = 4096;

#[derive(Default)]
pub struct Calc {
    /// Signals the exchange rates' refresh landing on disk.
    redraws: Option<std::sync::mpsc::Receiver<()>>,
}

impl Mode for Calc {
    /// Calc's rows come from the query, not from here; collecting starts
    /// refreshing the exchange rates in the background when they are stale.
    fn collect(&mut self) -> Result<Vec<Entry>> {
        self.redraws = rates::refresh_in_background();
        Ok(Vec::new())
    }

    fn redraws(&mut self) -> Option<std::sync::mpsc::Receiver<()>> {
        self.redraws.take()
    }
}

/// A result, written two ways for the locale.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Answer {
    /// For the row: grouped by threes (`1,234.5`), a repeating block in
    /// parentheses (`0.(3)`).
    pub shown: String,
    /// For stdout and the clipboard: no grouping (`1234.5`), rounded to 20
    /// significant digits where it does not end, so it pastes as a number.
    pub output: String,
    /// Whether `shown` is rounded rather than exact (`√2`, `pi`, money).
    pub approximate: bool,
    /// What the row says about the answer: which exchange rates converted
    /// it.
    pub note: Option<String>,
}

/// The stack calc evaluates on. The parser recurses per nesting level
/// ([`parser::MAX_DEPTH`] levels at most), and the UI thread's own stack is
/// 1 MB on Windows, so evaluation gets a thread of its own; the memory is
/// only reserved, and touched as deep as an expression goes.
const EVAL_STACK: usize = 16 * 1024 * 1024;

/// Runs `f` on a thread with [`EVAL_STACK`] of stack, or `None` when no
/// thread can be started.
fn on_eval_stack<T: Send>(f: impl FnOnce() -> Option<T> + Send) -> Option<T> {
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .name("pikr-calc".into())
            .stack_size(EVAL_STACK)
            .spawn_scoped(scope, f)
            .ok()?
            .join()
            .ok()?
    })
}

/// Evaluate `expr` with the locale's `separators`, or `None` when it is
/// empty, does not parse or has no finite value.
pub fn eval(expr: &str, separators: Separators, rates: &Rates) -> Option<Answer> {
    on_eval_stack(|| eval_here(expr, separators, rates))
}

/// [`eval`] on the current thread's stack.
fn eval_here(expr: &str, separators: Separators, rates: &Rates) -> Option<Answer> {
    let trimmed = expr.trim();
    if trimmed.is_empty() || trimmed.chars().count() > MAX_EXPR_LEN {
        return None;
    }
    let tokens = lexer::tokens(trimmed, separators)?;
    let (value, sources) = parser::evaluate(&tokens, rates)?;
    let (number, unit) = value.shown()?;
    let rendered = match &unit {
        Some(code) if value.dims == units::MONEY => {
            format::render_fixed(&number, rates::decimals(code), separators)?
        }
        _ => format::render(&number, separators)?,
    };
    let with_unit = |text: String| match &unit {
        Some(unit) => format!("{text} {unit}"),
        None => text,
    };
    Some(Answer {
        shown: with_unit(rendered.shown),
        output: with_unit(rendered.output),
        approximate: rendered.approximate,
        note: rates.note(&sources, std::time::SystemTime::now()),
    })
}

/// Precompute the result of every stored calc-history expression, once per
/// session. History strings are immutable while pikr runs (entries are
/// pushed only at Accept, after which the process exits), so rerank_calc
/// can read these instead of re-evaluating every expression per keystroke.
pub fn precompute(
    history: &[String],
    separators: Separators,
    rates: &Rates,
) -> HashMap<String, Answer> {
    on_eval_stack(|| {
        Some(
            history
                .iter()
                .filter_map(|expr| {
                    eval_here(expr, separators, rates).map(|answer| (expr.clone(), answer))
                })
                .collect(),
        )
    })
    .unwrap_or_default()
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

    fn out(expr: &str) -> Option<String> {
        eval(expr, EN, &Rates::default()).map(|a| a.output)
    }

    fn out_de(expr: &str) -> Option<String> {
        eval(expr, DE, &Rates::default()).map(|a| a.output)
    }

    #[test]
    fn empty_returns_none() {
        assert_eq!(out(""), None);
        assert_eq!(out("   "), None);
    }

    #[test]
    fn arithmetic_and_precedence() {
        assert_eq!(out("2 + 2").as_deref(), Some("4"));
        assert_eq!(out("10 * 5 - 3").as_deref(), Some("47"));
        assert_eq!(out("7/2").as_deref(), Some("3.5"));
        assert_eq!(out("1/3").as_deref(), Some("0.33333333333333333333"));
        assert_eq!(out("5/2").as_deref(), Some("2.5"));
        assert_eq!(out("6/2").as_deref(), Some("3"));
        assert_eq!(out("1/3*3").as_deref(), Some("1"));
        assert_eq!(out("0.1+0.2").as_deref(), Some("0.3"));
        assert_eq!(out("-3^2").as_deref(), Some("-9"));
        assert_eq!(out("2^-1").as_deref(), Some("0.5"));
        assert_eq!(out("2^3^2").as_deref(), Some("512"));
        assert_eq!(out("2**10").as_deref(), Some("1024"));
        assert_eq!(out("(1 + 2) * 3").as_deref(), Some("9"));
    }

    #[test]
    fn calculator_notation() {
        assert_eq!(out("5 × 3").as_deref(), Some("15"));
        assert_eq!(out("6 ÷ 4").as_deref(), Some("1.5"));
        assert_eq!(out("5 − 3").as_deref(), Some("2"));
        assert_eq!(out("2(3+4)").as_deref(), Some("14"));
        assert_eq!(out("(1+1)(2+2)").as_deref(), Some("8"));
        assert_eq!(out("1/2pi").as_deref(), out("1/(2pi)").as_deref());
        assert_eq!(out("5!").as_deref(), Some("120"));
        assert_eq!(out("15%").as_deref(), Some("0.15"));
        assert_eq!(out("50% * 80").as_deref(), Some("40"));
        assert_eq!(out("10 % 3").as_deref(), Some("1"));
        assert_eq!(out("√16").as_deref(), Some("4"));
        assert_eq!(out("0xff + 0b1").as_deref(), Some("256"));
    }

    #[test]
    fn constants_and_functions_by_their_plain_names() {
        assert_eq!(out("pi").as_deref(), Some("3.1415926535897932385"));
        assert_eq!(out("2π").as_deref(), Some("6.2831853071795864769"));
        assert_eq!(out("e").as_deref(), Some("2.7182818284590452354"));
        assert_eq!(out("sqrt(2)").as_deref(), Some("1.4142135623730950488"));
        assert_eq!(out("sqrt(16/9)").as_deref(), Some("1.3333333333333333333"));
        assert_eq!(out("8^(1/3)").as_deref(), Some("2"));
        assert_eq!(out("sin(pi/2)").as_deref(), Some("1"));
        assert_eq!(out("sin(30°)").as_deref(), Some("0.5"));
        assert_eq!(out("ln(e)").as_deref(), Some("1"));
        assert_eq!(out("log(1000)").as_deref(), Some("3"));
        assert_eq!(out("log(8, 2)").as_deref(), Some("3"));
        assert_eq!(out("abs(-3)").as_deref(), Some("3"));
        assert_eq!(out("floor(2.7)").as_deref(), Some("2"));
        assert_eq!(out("max(1, 5, 3)").as_deref(), Some("5"));
        assert_eq!(out("MIN(4; 2)").as_deref(), Some("2"));
    }

    #[test]
    fn big_integers_stay_exact() {
        assert_eq!(
            out("9999999999*9999999999").as_deref(),
            Some("99999999980000000001")
        );
        assert_eq!(out("25!").as_deref(), Some("15511210043330985984000000"));
    }

    #[test]
    fn no_value_is_no_result() {
        assert_eq!(out("1/0"), None);
        assert_eq!(out("sqrt(-1)"), None);
        assert_eq!(out("2 +"), None);
        assert_eq!(out("(1+2"), None);
        assert_eq!(out("zzz"), None);
        assert_eq!(out("1 2"), None);
        assert_eq!(out("1 000"), None);
        assert_eq!(out("max()"), None);
        assert_eq!(out("1 < 2"), None);
    }

    #[test]
    fn thousands_separators_follow_the_locale() {
        assert_eq!(out("1,000 + 1").as_deref(), Some("1001"));
        assert_eq!(out_de("1.000 + 1").as_deref(), Some("1001"));
        assert_eq!(out_de("1,5 * 2").as_deref(), Some("3"));
        assert_eq!(out_de("2,5 / 2").as_deref(), Some("1,25"));
        assert_eq!(out_de("max(1,5; 2)").as_deref(), Some("2"));
        // A space after the comma makes it an argument separator.
        assert_eq!(out("max(1, 234)").as_deref(), Some("234"));
    }

    #[test]
    fn the_row_groups_and_the_output_does_not() {
        let answer = eval("1234567.5 * 1", EN, &Rates::default()).unwrap();
        assert_eq!(answer.shown, "1,234,567.5");
        assert_eq!(answer.output, "1234567.5");
        let answer = eval("1234567,5 * 1", DE, &Rates::default()).unwrap();
        assert_eq!(answer.shown, "1.234.567,5");
        assert_eq!(answer.output, "1234567,5");
    }

    #[test]
    fn the_row_marks_repeats_and_approximations() {
        let third = eval("1/3", EN, &Rates::default()).unwrap();
        assert_eq!((third.shown.as_str(), third.approximate), ("0.(3)", false));
        let root2 = eval("√2", EN, &Rates::default()).unwrap();
        assert_eq!(
            (root2.shown.as_str(), root2.approximate),
            ("1.4142135623730950488", true)
        );
        let exact_trig = eval("sin(30°)", EN, &Rates::default()).unwrap();
        assert_eq!(
            (exact_trig.shown.as_str(), exact_trig.approximate),
            ("0.5", false)
        );
    }

    #[test]
    fn results_too_large_for_exact_maths_are_approximate() {
        let big = eval("2^99999", EN, &Rates::default()).unwrap();
        assert_eq!(big.output, "4.9950104650719225397e30102");
        assert!(big.approximate);
        assert_eq!(out("2^1000000"), None);
    }

    #[test]
    fn conversions_between_units_are_exact() {
        assert_eq!(out("6 ft to cm").as_deref(), Some("182.88 cm"));
        assert_eq!(out("5 lb in kg").as_deref(), Some("2.26796185 kg"));
        assert_eq!(out("1 gal -> L").as_deref(), Some("3.785411784 L"));
        assert_eq!(out("100 mph to kph").as_deref(), Some("160.9344 kph"));
        assert_eq!(out("1 GiB as MB").as_deref(), Some("1073.741824 MB"));
        assert_eq!(out("1 kWh to J").as_deref(), Some("3600000 J"));
        assert_eq!(
            out("5 km to mi").as_deref(),
            Some("3.1068559611866698481 mi")
        );
        let miles = eval("5 km to mi", EN, &Rates::default()).unwrap();
        assert_eq!(
            (miles.shown.as_str(), miles.approximate),
            ("3.1068559611866698481 mi", true)
        );
        assert_eq!(
            out("60 km/h to mph").as_deref(),
            Some("37.282271534240038177 mph")
        );
        assert_eq!(
            out("1 psi to kPa").as_deref(),
            Some("6.8947572931683613367 kPa")
        );
    }

    #[test]
    fn temperatures_convert_as_readings() {
        assert_eq!(out("20 °C to °F").as_deref(), Some("68 °F"));
        assert_eq!(out("98.6°F to °C").as_deref(), Some("37 °C"));
        assert_eq!(out("0 K in °C").as_deref(), Some("-273.15 °C"));
        assert_eq!(out("20 degC to kelvin").as_deref(), Some("293.15 kelvin"));
    }

    #[test]
    fn quantities_compute_and_keep_their_unit() {
        assert_eq!(out("5 km + 300 m").as_deref(), Some("5.3 km"));
        assert_eq!(out("2 * 3 ft").as_deref(), Some("6 ft"));
        assert_eq!(out("120 km / 2 h").as_deref(), Some("60 km/h"));
        assert_eq!(out("3 m * 4 m").as_deref(), Some("12 m·m"));
        assert_eq!(out("(3 m)^2").as_deref(), Some("9 m^2"));
        assert_eq!(out("sqrt(16 m^2)").as_deref(), Some("4 m"));
        assert_eq!(out("5 km / 1 m").as_deref(), Some("5000"));
        assert_eq!(out("floor(5.3 km)").as_deref(), Some("5 km"));
        assert_eq!(out("5 in").as_deref(), Some("5 in"));
        assert_eq!(out("5 in to cm").as_deref(), Some("12.7 cm"));
    }

    #[test]
    fn mismatched_units_are_no_result() {
        assert_eq!(out("5 km + 3 kg"), None);
        assert_eq!(out("5 km to kg"), None);
        assert_eq!(out("5 km to 2 mi"), None);
        assert_eq!(out("sin(3 m)"), None);
        assert_eq!(out("5 to km"), None);
        assert_eq!(out("2^(3 m)"), None);
    }

    #[test]
    fn money_converts_with_the_cached_rates() {
        let rates = rates::tests::sample_rates();
        let money = |expr: &str| eval(expr, EN, &rates);
        let euros = money("100 USD to EUR").unwrap();
        assert_eq!(euros.output, "92.49 EUR");
        assert!(euros.approximate);
        assert_eq!(euros.note.as_deref(), Some("rates: ECB 2026-11-12"));
        let output = |expr: &str| money(expr).map(|a| a.output);
        assert_eq!(output("$100 in £").as_deref(), Some("77.23 GBP"));
        assert_eq!(output("1000 jpy to eur").as_deref(), Some("6.04 EUR"));
        assert_eq!(
            money("10 € to ¥").map(|a| a.shown).as_deref(),
            Some("1,654 JPY")
        );
        assert_eq!(output("100 USD").as_deref(), Some("100.00 USD"));
        assert_eq!(output("5 USD + 3 EUR").as_deref(), Some("8.24 USD"));
        // No rate for it: no result, never a made-up one.
        assert_eq!(output("100 CHF to EUR"), None);
        assert_eq!(eval("100 USD to EUR", EN, &Rates::default()), None);
        // A plain result carries no rates note.
        assert_eq!(money("2+2").and_then(|a| a.note), None);
    }

    #[test]
    fn crypto_converts_with_the_cached_prices() {
        let rates = rates::tests::sample_rates();
        let output = |expr: &str| eval(expr, EN, &rates).map(|a| a.output);
        assert_eq!(output("1 BTC to EUR").as_deref(), Some("62110.00 EUR"));
        assert_eq!(output("1000 EUR in btc").as_deref(), Some("0.01610047 BTC"));
        assert_eq!(output("1000000 shib to €").as_deref(), Some("3.85 EUR"));
        let dollars = eval("1 eth in usd", EN, &rates).unwrap();
        assert_eq!(dollars.shown, "1,887.82 USD");
        let note = dollars.note.unwrap();
        assert!(
            note.starts_with("rates: ECB 2026-11-12, CoinGecko "),
            "{note}"
        );
        assert_eq!(output("1 XYZ to EUR"), None);
    }

    #[test]
    fn deep_nesting_is_refused_not_a_stack_overflow() {
        // The deepest input the limit lets through must fit calc's stack in a
        // debug build, where frames are largest; one level deeper is refused.
        let depth = parser::MAX_DEPTH;
        let deepest = format!("{}1 km{}", "(".repeat(depth - 1), ")".repeat(depth - 1));
        assert_eq!(out(&deepest).as_deref(), Some("1 km"));
        let refused = format!("{}1{}", "(".repeat(depth + 1), ")".repeat(depth + 1));
        assert_eq!(out(&refused), None);
        let negations = format!("{}1", "-".repeat(2000));
        assert_eq!(out(&negations), None);
    }

    #[test]
    fn over_cap_expression_rejected() {
        let long = format!("1{}", " + 1".repeat(MAX_EXPR_LEN / 4));
        assert!(long.chars().count() > MAX_EXPR_LEN);
        assert_eq!(out(&long), None);
        // Whitespace padding does not trip the cap: it counts the trimmed
        // expression.
        let padded = format!("{}1", " ".repeat(MAX_EXPR_LEN + 1));
        assert_eq!(out(&padded).as_deref(), Some("1"));
    }

    #[test]
    fn at_cap_expression_still_evaluates() {
        let expr: String = format!("1{}", " + 1".repeat((MAX_EXPR_LEN - 1) / 4));
        assert!(expr.chars().count() <= MAX_EXPR_LEN);
        assert_eq!(out(&expr).as_deref(), Some("1024"));
    }

    #[test]
    fn precompute_keeps_only_what_evaluates() {
        let input: Vec<String> = vec!["1+1".into(), "zzz".into(), "7/2".into()];
        let map = precompute(&input, EN, &Rates::default());
        assert_eq!(map.len(), 2);
        assert_eq!(map.get("1+1").map(|a| a.output.as_str()), Some("2"));
        assert!(!map.contains_key("zzz"));
        for (expr, answer) in &map {
            assert_eq!(Some(answer.clone()), eval(expr, EN, &Rates::default()));
        }
    }
}
