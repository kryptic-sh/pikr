//! calc mode — reactive expression evaluator.
//!
//! Unlike the other modes, `Calc::collect()` returns an empty list. Calc is
//! driven by `AppState::rerank`, which short-circuits when `cli_mode == Calc`
//! and calls [`eval`] against the live query, producing a single synthetic
//! `Entry` per evaluation.
//!
//! The evaluator is pikr's own ([`lexer`], [`parser`], [`number`]): numbers
//! are read and written with the locale's separators, integers stay exact
//! while they fit, and only a finite result is ever shown.

mod lexer;
mod locale;
mod number;
mod parser;

use std::collections::HashMap;

use super::{Entry, Mode};
use anyhow::Result;

pub use locale::{separators, validate as validate_separators};
pub use number::Separators;

/// Hard cap on expression length, in characters. The parser's own nesting
/// limit is what keeps it off the end of the stack; this only bounds the
/// work a pasted wall of text costs on every keystroke. Expressions over the
/// cap are rejected the way a parse error is (no result row).
const MAX_EXPR_LEN: usize = 4096;

#[derive(Default)]
pub struct Calc;

impl Mode for Calc {
    fn collect(&mut self) -> Result<Vec<Entry>> {
        Ok(Vec::new())
    }
}

/// A result, written two ways for the locale.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Answer {
    /// For the row: grouped by threes (`1,234.5`).
    pub shown: String,
    /// For stdout and the clipboard: no grouping (`1234.5`), so it pastes
    /// as a number.
    pub output: String,
}

/// Evaluate `expr` with the locale's `separators`, or `None` when it is
/// empty, does not parse or has no finite value.
pub fn eval(expr: &str, separators: Separators) -> Option<Answer> {
    let trimmed = expr.trim();
    if trimmed.is_empty() || trimmed.chars().count() > MAX_EXPR_LEN {
        return None;
    }
    let tokens = lexer::tokens(trimmed, separators)?;
    let value = parser::evaluate(&tokens)?;
    Some(Answer {
        shown: separators.shown(value),
        output: separators.plain(value),
    })
}

/// Precompute the result of every stored calc-history expression, once per
/// session. History strings are immutable while pikr runs (entries are
/// pushed only at Accept, after which the process exits), so rerank_calc
/// can read these instead of re-evaluating every expression per keystroke.
pub fn precompute(history: &[String], separators: Separators) -> HashMap<String, Answer> {
    history
        .iter()
        .filter_map(|expr| eval(expr, separators).map(|answer| (expr.clone(), answer)))
        .collect()
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
        eval(expr, EN).map(|a| a.output)
    }

    fn out_de(expr: &str) -> Option<String> {
        eval(expr, DE).map(|a| a.output)
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
        assert_eq!(out("1/3").as_deref(), Some("0.333333333333333"));
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
        assert_eq!(out("5!").as_deref(), Some("120"));
        assert_eq!(out("15%").as_deref(), Some("0.15"));
        assert_eq!(out("50% * 80").as_deref(), Some("40"));
        assert_eq!(out("10 % 3").as_deref(), Some("1"));
        assert_eq!(out("√16").as_deref(), Some("4"));
        assert_eq!(out("0xff + 0b1").as_deref(), Some("256"));
    }

    #[test]
    fn constants_and_functions_by_their_plain_names() {
        assert_eq!(out("pi").as_deref(), Some("3.14159265358979"));
        assert_eq!(out("2π").as_deref(), Some("6.28318530717959"));
        assert_eq!(out("e").as_deref(), Some("2.71828182845905"));
        assert_eq!(out("sqrt(2)").as_deref(), Some("1.4142135623731"));
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
        let answer = eval("1234567.5 * 1", EN).unwrap();
        assert_eq!(answer.shown, "1,234,567.5");
        assert_eq!(answer.output, "1234567.5");
        let answer = eval("1234567,5 * 1", DE).unwrap();
        assert_eq!(answer.shown, "1.234.567,5");
        assert_eq!(answer.output, "1234567,5");
    }

    #[test]
    fn deep_nesting_is_refused_not_a_stack_overflow() {
        // Over the parser's nesting limit, well under the length cap: on an
        // unbounded recursive parser this shape overflows the 1 MB Windows
        // main-thread stack.
        let deep = format!("{}1{}", "(".repeat(2000), ")".repeat(2000));
        assert_eq!(out(&deep), None);
        let negations = format!("{}1", "-".repeat(2000));
        assert_eq!(out(&negations), None);
        let shallow = format!("{}1{}", "(".repeat(100), ")".repeat(100));
        assert_eq!(out(&shallow).as_deref(), Some("1"));
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
        let map = precompute(&input, EN);
        assert_eq!(map.len(), 2);
        assert_eq!(map.get("1+1").map(|a| a.output.as_str()), Some("2"));
        assert!(!map.contains_key("zzz"));
        for (expr, answer) in &map {
            assert_eq!(Some(answer.clone()), eval(expr, EN));
        }
    }
}
