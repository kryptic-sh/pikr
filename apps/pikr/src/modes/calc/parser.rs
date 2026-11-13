//! Evaluates calc's tokens by precedence, lowest first:
//!
//! ```text
//! input    = sum (("to" | "in" | "as" | "->") sum)?          a conversion
//! sum      = product (("+" | "-") product)*
//! product  = term (("*" | "/" | "%") term)*
//! term     = unary unary*                                juxtaposition multiplies
//! unary    = ("+" | "-" | "√") unary | power
//! power    = postfix ("^" unary)?                        right-associative
//! postfix  = primary ("!" | "%" | "°" | "°C" | "°F")*
//! primary  = number | constant | unit | name "(" sum ("," sum)* ")" | "(" sum ")"
//! ```
//!
//! So `-3^2` is -9, `2^-1` is 0.5, `2^3^2` is 512, and `2pi`, `3(4+5)` and
//! `5 km` multiply (two bare numbers do not: `1 2` is no result).
//! Juxtaposition binds tighter than `*` and `/`, as in unit notation:
//! `120 km / 2 h` is 60 km/h and `1/2pi` is 1/(2π). `%` after
//! a value is a percentage (`15%` is 0.15) unless another value follows it,
//! which makes it the remainder (`10 % 3` is 1).

use std::cmp::Ordering;

use astro_float::BigFloat;
use num_rational::BigRational;

use super::lexer::Token;
use super::number::{FloatFn, Num};
use super::rates::{Rates, Source};
use super::units::{self, Unit};
use super::value::Value;

/// How deep parentheses, function calls and prefix operators may nest. The
/// parser recurses several frames per level (about 20 KB of stack each in a
/// debug build), so evaluation runs on a stack of its own
/// ([`super::EVAL_STACK`]); the test
/// `deep_nesting_is_refused_not_a_stack_overflow` evaluates an input this
/// deep. No real expression nests this far.
pub const MAX_DEPTH: usize = 256;

/// The words that ask for a conversion (`5 km to mi`).
const CONVERSION_WORDS: [&str; 3] = ["to", "in", "as"];

/// The value of `tokens` and the rate sources its currencies came from, or
/// `None` when they are not one complete expression or it has no finite
/// value.
///
/// A conversion splits at the last `to`/`in`/`as`/`->` outside parentheses
/// where both sides evaluate and the right side is one unit of the left
/// side's kind; failing that, the whole input is one expression (so `5 in`
/// is five inches).
pub fn evaluate(tokens: &[Token], rates: &Rates) -> Option<(Value, Vec<Source>)> {
    let mut depth = 0usize;
    let mut splits = Vec::new();
    for (i, token) in tokens.iter().enumerate() {
        match token {
            Token::LParen => depth += 1,
            Token::RParen => depth = depth.saturating_sub(1),
            Token::Arrow if depth == 0 => splits.push(i),
            Token::Ident(word) if depth == 0 && CONVERSION_WORDS.contains(&word.as_str()) => {
                splits.push(i);
            }
            _ => {}
        }
    }
    for &i in splits.iter().rev() {
        let (Some((from, mut sources)), Some((to, more))) = (
            expression(&tokens[..i], rates),
            expression(&tokens[i + 1..], rates),
        ) else {
            continue;
        };
        if let Some(converted) = from.convert(&to) {
            sources.extend(more);
            return Some((converted, sources));
        }
    }
    expression(tokens, rates)
}

/// The value of `tokens` as one expression, and its rate sources.
fn expression(tokens: &[Token], rates: &Rates) -> Option<(Value, Vec<Source>)> {
    if tokens.is_empty() {
        return None;
    }
    let mut parser = Parser {
        tokens,
        rates,
        sources: Vec::new(),
        at: 0,
        depth: 0,
    };
    let value = parser.sum()?;
    (parser.at == tokens.len()).then_some((value, parser.sources))
}

struct Parser<'a> {
    tokens: &'a [Token],
    rates: &'a Rates,
    /// Where the currencies read so far got their rates.
    sources: Vec<Source>,
    at: usize,
    depth: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.at)
    }

    fn eat(&mut self, token: &Token) -> bool {
        let found = self.peek() == Some(token);
        if found {
            self.at += 1;
        }
        found
    }

    /// Runs `f` one nesting level deeper, refusing past [`MAX_DEPTH`].
    fn nested(&mut self, f: impl FnOnce(&mut Self) -> Option<Value>) -> Option<Value> {
        if self.depth == MAX_DEPTH {
            return None;
        }
        self.depth += 1;
        let value = f(self);
        self.depth -= 1;
        value
    }

    fn sum(&mut self) -> Option<Value> {
        let mut value = self.product()?;
        loop {
            if self.eat(&Token::Plus) {
                value = value.add(&self.product()?)?;
            } else if self.eat(&Token::Minus) {
                value = value.sub(&self.product()?)?;
            } else {
                return Some(value);
            }
        }
    }

    fn product(&mut self) -> Option<Value> {
        let mut value = self.term()?;
        loop {
            if self.eat(&Token::Times) {
                value = value.mul(&self.term()?)?;
            } else if self.eat(&Token::Divide) {
                value = value.div(&self.term()?)?;
            } else if self.eat(&Token::Percent) {
                // `postfix` left this `%` because a value follows it.
                value = value.rem(&self.term()?)?;
            } else {
                return Some(value);
            }
        }
    }

    /// Values written side by side, multiplied (`2 h`, `3(4+5)`).
    fn term(&mut self) -> Option<Value> {
        let mut value = self.unary()?;
        while self.peek().is_some_and(multiplies_by_juxtaposition) {
            value = value.mul(&self.unary()?)?;
        }
        Some(value)
    }

    fn unary(&mut self) -> Option<Value> {
        if self.eat(&Token::Minus) {
            self.nested(|p| Some(p.unary()?.neg()))
        } else if self.eat(&Token::Plus) {
            self.nested(Self::unary)
        } else if self.eat(&Token::Root) {
            self.nested(|p| p.unary()?.root(2))
        } else {
            self.power()
        }
    }

    fn power(&mut self) -> Option<Value> {
        let base = self.postfix()?;
        if self.eat(&Token::Power) {
            let exponent = self.nested(Self::unary)?;
            base.pow(&exponent)
        } else {
            Some(base)
        }
    }

    fn postfix(&mut self) -> Option<Value> {
        let mut value = self.primary()?;
        loop {
            if self.eat(&Token::Bang) {
                value = value.factorial()?;
            } else if let Some(scale) = self.temperature_scale() {
                value = value.mul(&scale)?;
            } else if self.eat(&Token::Degree) {
                let degree = Num::pi()?.div(&Num::int(180))?;
                value = value.mul(&Value::plain(degree))?;
            } else if self.peek() == Some(&Token::Percent)
                && !self.tokens.get(self.at + 1).is_some_and(starts_value)
            {
                self.at += 1;
                value = value.div(&Value::plain(Num::int(100)))?;
            } else {
                return Some(value);
            }
        }
    }

    /// `°C` or `°F` at the cursor, as one degree of that scale.
    fn temperature_scale(&mut self) -> Option<Value> {
        let (Some(Token::Degree), Some(Token::Ident(letter))) =
            (self.tokens.get(self.at), self.tokens.get(self.at + 1))
        else {
            return None;
        };
        let name = format!("°{letter}");
        let unit = units::lookup(&name)?;
        self.at += 2;
        Some(Value::unit(&name, unit))
    }

    fn primary(&mut self) -> Option<Value> {
        if let Some(scale) = self.temperature_scale() {
            return Some(scale);
        }
        match self.peek()?.clone() {
            Token::Num(n) => {
                self.at += 1;
                Some(Value::plain(Num::Exact(n)))
            }
            Token::LParen => {
                self.at += 1;
                let value = self.nested(Self::sum)?;
                self.eat(&Token::RParen).then_some(value)
            }
            Token::Ident(name) => {
                self.at += 1;
                // A currency symbol written before its amount (`$100`).
                if !name.chars().any(char::is_alphanumeric)
                    && let Some(Token::Num(amount)) = self.peek().cloned()
                {
                    self.at += 1;
                    return self.currency(&name)?.mul(&Value::plain(Num::Exact(amount)));
                }
                if self.eat(&Token::LParen) {
                    let args = self.arguments()?;
                    call(&name.to_lowercase(), &args)
                } else {
                    constant(&name.to_lowercase())
                        .map(Value::plain)
                        .or_else(|| units::lookup(&name).map(|unit| Value::unit(&name, unit)))
                        .or_else(|| self.currency(&name))
                }
            }
            _ => None,
        }
    }

    /// One unit of the currency `name` (`USD`, `$`, `BTC`), shown by its
    /// code.
    fn currency(&mut self, name: &str) -> Option<Value> {
        let (code, euros, source) = self.rates.currency(name)?;
        if !self.sources.contains(&source) {
            self.sources.push(source);
        }
        let unit = Unit {
            dims: units::MONEY,
            factor: euros,
            zero: None,
        };
        Some(Value::unit(&code, unit))
    }

    /// A call's arguments, after its `(`, through its `)`.
    fn arguments(&mut self) -> Option<Vec<Value>> {
        let mut args = vec![self.nested(Self::sum)?];
        while self.eat(&Token::Comma) {
            args.push(self.nested(Self::sum)?);
        }
        self.eat(&Token::RParen).then_some(args)
    }
}

/// Whether `token` begins a value, which makes the `%` before it a
/// remainder rather than a percentage.
fn starts_value(token: &Token) -> bool {
    matches!(token, Token::Num(_)) || multiplies_by_juxtaposition(token)
}

/// Whether a value written straight after another multiplies it (`2pi`,
/// `3(4+5)`, `2√9`, `5 km`). A bare number does not: `1 000`, typed with a
/// space for a thousands separator, must be no result, not 0.
fn multiplies_by_juxtaposition(token: &Token) -> bool {
    matches!(token, Token::Ident(_) | Token::LParen | Token::Root)
}

fn constant(name: &str) -> Option<Num> {
    match name {
        "pi" | "π" => Num::pi(),
        "tau" | "τ" => Num::pi()?.mul(&Num::int(2)),
        "e" => Num::e(),
        _ => None,
    }
}

/// A function's value. Trigonometry is in radians; write `sin(30°)` for
/// degrees.
fn call(name: &str, args: &[Value]) -> Option<Value> {
    let one = || match args {
        [x] => Some(x),
        _ => None,
    };
    let float = |f: FloatFn| one()?.apply(f);
    let ten = || Value::plain(Num::int(10));
    match name {
        "sqrt" => one()?.root(2),
        "cbrt" => one()?.root(3),
        "exp" => float(BigFloat::exp),
        "ln" => float(BigFloat::ln),
        "log2" => one()?.log(&Value::plain(Num::int(2))),
        "log10" => one()?.log(&ten()),
        "log" => match args {
            [x] => x.log(&ten()),
            [x, base] => x.log(base),
            _ => None,
        },
        "sin" => float(BigFloat::sin),
        "cos" => float(BigFloat::cos),
        "tan" => float(BigFloat::tan),
        "asin" => float(BigFloat::asin),
        "acos" => float(BigFloat::acos),
        "atan" => float(BigFloat::atan),
        "sinh" => float(BigFloat::sinh),
        "cosh" => float(BigFloat::cosh),
        "tanh" => float(BigFloat::tanh),
        "abs" => Some(one()?.abs()),
        "floor" => one()?.whole_with(BigRational::floor),
        "ceil" => one()?.whole_with(BigRational::ceil),
        "round" => one()?.whole_with(BigRational::round),
        "trunc" => one()?.whole_with(BigRational::trunc),
        "min" | "max" => {
            let (first, rest) = args.split_first()?;
            let wanted = if name == "min" {
                Ordering::Less
            } else {
                Ordering::Greater
            };
            rest.iter().try_fold(first.clone(), |best, n| {
                Some(if n.cmp(&best)? == wanted {
                    n.clone()
                } else {
                    best
                })
            })
        }
        _ => None,
    }
}
