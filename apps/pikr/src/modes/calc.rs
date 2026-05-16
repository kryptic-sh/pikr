//! calc mode — reactive expression evaluator.
//!
//! Unlike the other modes, `Calc::collect()` returns an empty list. Calc is
//! driven by `AppState::rerank`, which short-circuits when `cli_mode == Calc`
//! and calls [`eval`] against the live query, producing a single synthetic
//! `Entry` per evaluation.

use super::{Entry, Mode};
use anyhow::Result;

#[derive(Default)]
pub struct Calc;

impl Mode for Calc {
    fn name(&self) -> &'static str {
        "calc"
    }

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
    fn boolean_supported() {
        assert_eq!(eval("1 < 2"), Some("true".to_string()));
    }
}
