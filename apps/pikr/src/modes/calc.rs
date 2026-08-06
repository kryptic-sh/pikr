//! calc mode — reactive expression evaluator.
//!
//! Unlike the other modes, `Calc::collect()` returns an empty list. Calc is
//! driven by `AppState::rerank`, which short-circuits when `cli_mode == Calc`
//! and calls [`eval`] against the live query, producing a single synthetic
//! `Entry` per evaluation.

use super::{Entry, Mode};
use anyhow::Result;

/// Hard cap on expression length. evalexpr's evaluator recurses one frame per
/// tree level (and its `Drop` recurses too), with no depth limit of its own:
/// a ~50k-deep nested-paren expression overflows the 8 MB main-thread stack
/// and aborts the process (SIGSEGV). A length cap bounds the depth — the
/// worst case is a paren-heavy expression whose depth is proportional to its
/// length — and 4096 chars is far beyond any real calculation. Expressions
/// over the cap are rejected the same way a parse error is (no result row),
/// so the expression never reaches evalexpr and never lands in calc history
/// to be re-evaluated on every subsequent keystroke.
const MAX_EXPR_LEN: usize = 4096;

#[derive(Default)]
pub struct Calc;

impl Mode for Calc {
    fn collect(&mut self) -> Result<Vec<Entry>> {
        Ok(Vec::new())
    }
}

/// Evaluate `expr` and return a display string for the result, or `None` if
/// the expression is empty or fails to parse / evaluate.
pub fn eval(expr: &str) -> Option<String> {
    let trimmed = expr.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.chars().count() > MAX_EXPR_LEN {
        // Reject before evalexpr: see the const doc for why an unbounded
        // expression can abort the process.
        return None;
    }
    let value = evalexpr::eval(trimmed).ok()?;
    match value {
        evalexpr::Value::Int(i) => Some(i.to_string()),
        evalexpr::Value::Float(f) => Some(format_float(f)),
        evalexpr::Value::String(s) => Some(s),
        evalexpr::Value::Boolean(b) => Some(b.to_string()),
        // Tuple / Empty / etc. aren't useful pickable values.
        _ => None,
    }
}

fn format_float(f: f64) -> String {
    if f.fract() == 0.0 && f.is_finite() && f.abs() < 1e16 {
        // Whole-number floats render without a trailing `.0` for readability.
        format!("{}", f as i64)
    } else {
        format!("{f}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_returns_none() {
        assert_eq!(eval(""), None);
        assert_eq!(eval("   "), None);
    }

    #[test]
    fn integer_arith() {
        assert_eq!(eval("2 + 2"), Some("4".to_string()));
        assert_eq!(eval("10 * 5 - 3"), Some("47".to_string()));
    }

    #[test]
    fn float_arith_clean() {
        assert_eq!(eval("10.0 / 4.0"), Some("2.5".to_string()));
        // 6.0 / 2.0 = 3 (whole) — no trailing .0
        assert_eq!(eval("6.0 / 2.0"), Some("3".to_string()));
    }

    #[test]
    fn parse_error_returns_none() {
        assert_eq!(eval("2 +"), None);
        assert_eq!(eval("zzz"), None);
    }

    #[test]
    fn over_cap_expression_rejected() {
        // The audit repro shape: deeply nested parens. Over the cap the
        // expression returns None without ever reaching evalexpr's recursive
        // evaluator — on the uncapped code a ~50k-deep variant aborted the
        // process with a stack overflow, and this smaller one evaluated fine
        // (returned Some("1")).
        let deep: String = format!("{}1{}", "(".repeat(MAX_EXPR_LEN), ")".repeat(MAX_EXPR_LEN));
        assert_eq!(eval(&deep), None);
        // Whitespace padding does not trip the cap — it counts the TRIMMED
        // expression, so a short expression surrounded by spaces still
        // evaluates (and is not a stack risk).
        let padded = format!("{}1", " ".repeat(MAX_EXPR_LEN + 1));
        assert_eq!(eval(&padded), Some("1".to_string()));
    }

    #[test]
    fn at_cap_expression_still_evaluates() {
        // Boundary: an expression at (or just under) the cap is not rejected.
        let expr: String = format!("1{}", " + 1".repeat((MAX_EXPR_LEN - 1) / 4));
        assert!(expr.chars().count() <= MAX_EXPR_LEN);
        assert_eq!(eval(&expr), Some("1024".to_string()));
    }

    #[test]
    fn boolean_supported() {
        assert_eq!(eval("1 < 2"), Some("true".to_string()));
    }
}
