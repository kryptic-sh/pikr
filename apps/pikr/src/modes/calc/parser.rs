//! Evaluates calc's tokens by precedence, lowest first:
//!
//! ```text
//! sum      = product (("+" | "-") product)*
//! product  = unary (("*" | "/" | "%") unary | unary)*   juxtaposition multiplies
//! unary    = ("+" | "-" | "√") unary | power
//! power    = postfix ("^" unary)?                        right-associative
//! postfix  = primary ("!" | "%" | "°")*
//! primary  = number | constant | name "(" sum ("," sum)* ")" | "(" sum ")"
//! ```
//!
//! So `-3^2` is -9, `2^-1` is 0.5, `2^3^2` is 512, and `2pi` and `3(4+5)`
//! multiply (two bare numbers do not: `1 2` is no result). `%` after a value is a percentage (`15%` is 0.15) unless another
//! value follows it, which makes it the remainder (`10 % 3` is 1).

use std::cmp::Ordering;

use astro_float::BigFloat;
use num_rational::BigRational;

use super::lexer::Token;
use super::number::{FloatFn, Num};

/// How deep parentheses, function calls and prefix operators may nest. The
/// parser recurses a few frames per level, and calc evaluates on the UI
/// thread (1 MB of stack on Windows): this keeps any input far from it.
const MAX_DEPTH: usize = 256;

/// The value of `tokens`, or `None` when they are not one complete
/// expression or it has no finite value.
pub fn evaluate(tokens: &[Token]) -> Option<Num> {
    let mut parser = Parser {
        tokens,
        at: 0,
        depth: 0,
    };
    let value = parser.sum()?;
    (parser.at == tokens.len()).then_some(value)
}

struct Parser<'a> {
    tokens: &'a [Token],
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
    fn nested(&mut self, f: impl FnOnce(&mut Self) -> Option<Num>) -> Option<Num> {
        if self.depth == MAX_DEPTH {
            return None;
        }
        self.depth += 1;
        let value = f(self);
        self.depth -= 1;
        value
    }

    fn sum(&mut self) -> Option<Num> {
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

    fn product(&mut self) -> Option<Num> {
        let mut value = self.unary()?;
        loop {
            if self.eat(&Token::Times) {
                value = value.mul(&self.unary()?)?;
            } else if self.eat(&Token::Divide) {
                value = value.div(&self.unary()?)?;
            } else if self.eat(&Token::Percent) {
                // `postfix` left this `%` because a value follows it.
                value = value.rem(&self.unary()?)?;
            } else if self.peek().is_some_and(multiplies_by_juxtaposition) {
                value = value.mul(&self.unary()?)?;
            } else {
                return Some(value);
            }
        }
    }

    fn unary(&mut self) -> Option<Num> {
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

    fn power(&mut self) -> Option<Num> {
        let base = self.postfix()?;
        if self.eat(&Token::Power) {
            let exponent = self.nested(Self::unary)?;
            base.pow(&exponent)
        } else {
            Some(base)
        }
    }

    fn postfix(&mut self) -> Option<Num> {
        let mut value = self.primary()?;
        loop {
            if self.eat(&Token::Bang) {
                value = value.factorial()?;
            } else if self.eat(&Token::Degree) {
                value = value.mul(&Num::pi()?)?.div(&Num::int(180))?;
            } else if self.peek() == Some(&Token::Percent)
                && !self.tokens.get(self.at + 1).is_some_and(starts_value)
            {
                self.at += 1;
                value = value.div(&Num::int(100))?;
            } else {
                return Some(value);
            }
        }
    }

    fn primary(&mut self) -> Option<Num> {
        match self.peek()?.clone() {
            Token::Num(n) => {
                self.at += 1;
                Some(Num::Exact(n))
            }
            Token::LParen => {
                self.at += 1;
                let value = self.nested(Self::sum)?;
                self.eat(&Token::RParen).then_some(value)
            }
            Token::Ident(name) => {
                self.at += 1;
                if self.eat(&Token::LParen) {
                    let args = self.arguments()?;
                    call(&name.to_lowercase(), &args)
                } else {
                    constant(&name.to_lowercase())
                }
            }
            _ => None,
        }
    }

    /// A call's arguments, after its `(`, through its `)`.
    fn arguments(&mut self) -> Option<Vec<Num>> {
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
/// `3(4+5)`, `2√9`). A bare number does not: `1 000`, typed with a space for
/// a thousands separator, must be no result, not 0.
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
fn call(name: &str, args: &[Num]) -> Option<Num> {
    let one = || match args {
        [x] => Some(x),
        _ => None,
    };
    let float = |f: FloatFn| one()?.apply(f);
    match name {
        "sqrt" => one()?.root(2),
        "cbrt" => one()?.root(3),
        "exp" => float(BigFloat::exp),
        "ln" => float(BigFloat::ln),
        "log2" => one()?.log(&Num::int(2)),
        "log10" => one()?.log(&Num::int(10)),
        "log" => match args {
            [x] => x.log(&Num::int(10)),
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
