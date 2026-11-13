//! Splits a calc expression into tokens, reading numbers the locale's way.

use num_bigint::BigInt;
use num_rational::BigRational;
use num_traits::Num as _;

use super::format::Separators;
use super::number::decimal;

#[derive(Clone, Debug, PartialEq)]
pub enum Token {
    /// A literal, always exact (`0.1` is one tenth).
    Num(BigRational),
    Ident(String),
    Plus,
    Minus,
    Times,
    Divide,
    Power,
    /// `%`: a percentage after a value, the remainder between two.
    Percent,
    /// `!`, the factorial.
    Bang,
    /// `°`: the value is in degrees.
    Degree,
    /// `√`, a square root of what follows.
    Root,
    LParen,
    RParen,
    /// Between a function's arguments.
    Comma,
    /// `->` or `→`: a conversion (`5 km -> mi`).
    Arrow,
}

/// The tokens of `text`, or `None` at a character calc does not read.
pub fn tokens(text: &str, seps: Separators) -> Option<Vec<Token>> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(&c) = chars.get(i) {
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if c.is_ascii_digit() || (c == seps.decimal && next_is_digit(&chars, i)) {
            let (num, used) = number(&chars[i..], seps)?;
            out.push(Token::Num(num));
            i += used;
            continue;
        }
        if c.is_alphabetic() {
            let start = i;
            while chars
                .get(i)
                .is_some_and(|&c| c.is_alphanumeric() || c == '_')
            {
                i += 1;
            }
            out.push(Token::Ident(chars[start..i].iter().collect()));
            continue;
        }
        let token = match c {
            '+' => Token::Plus,
            '-' if chars.get(i + 1) == Some(&'>') => {
                i += 1;
                Token::Arrow
            }
            '→' => Token::Arrow,
            '$' | '€' | '£' | '¥' | '₹' => Token::Ident(c.to_string()),
            '-' | '−' => Token::Minus,
            '*' if chars.get(i + 1) == Some(&'*') => {
                i += 1;
                Token::Power
            }
            '*' | '×' | '·' | '⋅' => Token::Times,
            '/' | '÷' | '∕' => Token::Divide,
            '^' => Token::Power,
            '%' => Token::Percent,
            '!' => Token::Bang,
            '°' => Token::Degree,
            '√' => Token::Root,
            '(' => Token::LParen,
            ')' => Token::RParen,
            ';' => Token::Comma,
            c if c == seps.argument() || c == ',' => Token::Comma,
            _ => return None,
        };
        out.push(token);
        i += 1;
    }
    Some(out)
}

fn next_is_digit(chars: &[char], i: usize) -> bool {
    chars.get(i + 1).is_some_and(char::is_ascii_digit)
}

/// The number at the start of `chars` and how many characters it used.
///
/// A group separator counts only where it is followed by exactly three
/// digits and the digits before it started with a run of one to three (so
/// in English `1,000` is a thousand, while `1,5` and `1, 500` are two
/// arguments). `_` groups too, in any locale. Where `,` is the decimal mark,
/// a `.` that cannot be a group is read as a decimal point, so `3.14` still
/// works in German.
fn number(chars: &[char], seps: Separators) -> Option<(BigRational, usize)> {
    if let Some(radix) = radix_prefix(chars) {
        let mut i = 2;
        let mut digits = String::new();
        while let Some(&c) = chars.get(i) {
            if c.is_digit(radix) {
                digits.push(c);
            } else if c != '_' {
                break;
            }
            i += 1;
        }
        let value = BigInt::from_str_radix(&digits, radix).ok()?;
        return Some((BigRational::from_integer(value), i));
    }

    let mut whole = String::new();
    let mut fractional = String::new();
    let mut i = 0;
    let mut run = 0;
    let mut grouped = false;
    let mut fraction = false;
    while let Some(&c) = chars.get(i) {
        if c.is_ascii_digit() {
            if fraction {
                fractional.push(c);
            } else {
                whole.push(c);
            }
            run += 1;
            i += 1;
        } else if !fraction
            && is_group_char(c, seps)
            && (grouped || (1..=3).contains(&run))
            && is_group(&chars[i + 1..])
        {
            grouped = true;
            run = 0;
            i += 1;
        } else if !fraction
            && (c == seps.decimal || (c == '.' && seps.decimal == ','))
            && next_is_digit_or_end(chars, i)
        {
            fraction = true;
            i += 1;
        } else {
            break;
        }
    }

    let mut exponent = 0;
    if matches!(chars.get(i), Some('e' | 'E')) {
        let sign = chars.get(i + 1).filter(|c| matches!(c, '+' | '-'));
        let first = i + 1 + usize::from(sign.is_some());
        if chars.get(first).is_some_and(char::is_ascii_digit) {
            let mut digits = String::new();
            i = first;
            while let Some(&c) = chars.get(i).filter(|c| c.is_ascii_digit()) {
                digits.push(c);
                i += 1;
            }
            // Too many digits to parse is past any exponent calc allows.
            let magnitude: i64 = digits.parse().ok()?;
            exponent = if sign == Some(&'-') {
                -magnitude
            } else {
                magnitude
            };
        }
    }
    Some((decimal(&whole, &fractional, exponent)?, i))
}

/// The radix of a `0x`, `0b` or `0o` literal.
fn radix_prefix(chars: &[char]) -> Option<u32> {
    let radix = match chars.get(..2)? {
        ['0', 'x' | 'X'] => 16,
        ['0', 'b' | 'B'] => 2,
        ['0', 'o' | 'O'] => 8,
        _ => return None,
    };
    chars.get(2).filter(|c| c.is_digit(radix)).map(|_| radix)
}

/// Whether `c` separates digit groups: the locale's separator, `_` in any
/// locale, and a plain space where the locale groups with a space (French's
/// U+202F, which nobody types).
fn is_group_char(c: char, seps: Separators) -> bool {
    c == seps.group || c == '_' || (c == ' ' && seps.group.is_whitespace())
}

/// Whether `after` (what follows a group separator) is exactly three digits.
fn is_group(after: &[char]) -> bool {
    after.len() >= 3
        && after[..3].iter().all(char::is_ascii_digit)
        && !after.get(3).is_some_and(char::is_ascii_digit)
}

/// A decimal mark needs a digit after it, or the end of the input (`3.`
/// while the user is still typing). Anything else ends the number, so `1, 5`
/// is two arguments even where `,` is the decimal mark.
fn next_is_digit_or_end(chars: &[char], i: usize) -> bool {
    chars.get(i + 1).is_none_or(char::is_ascii_digit)
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

    fn num(text: &str, seps: Separators) -> Option<BigRational> {
        match tokens(text, seps)?.as_slice() {
            [Token::Num(n)] => Some(n.clone()),
            _ => None,
        }
    }

    fn q(n: i64, d: i64) -> BigRational {
        BigRational::new(n.into(), d.into())
    }

    #[test]
    fn groups_need_three_digits_after_them() {
        assert_eq!(num("1,000", EN), Some(q(1000, 1)));
        assert_eq!(num("12,345,678", EN), Some(q(12_345_678, 1)));
        assert_eq!(num("1,000.25", EN), Some(q(4001, 4)));
        assert_eq!(num("1_000", EN), Some(q(1000, 1)));
        // Not groups: these are separate tokens, never one number.
        assert_eq!(num("1,5", EN), None);
        assert_eq!(num("1,0000", EN), None);
        assert_eq!(num("1234,567", EN), None);
    }

    #[test]
    fn a_comma_decimal_locale_reads_both_ways_it_can() {
        assert_eq!(num("1.000,5", DE), Some(q(2001, 2)));
        assert_eq!(num("1,5", DE), Some(q(3, 2)));
        assert_eq!(num("1.000", DE), Some(q(1000, 1)));
        // A '.' that cannot group is a decimal point there, so `2.75`
        // still reads as typed.
        assert_eq!(num("2.75", DE), Some(q(11, 4)));
    }

    #[test]
    fn a_space_groups_where_the_locale_groups_with_one() {
        let french = Separators {
            decimal: ',',
            group: '\u{202f}',
        };
        assert_eq!(num("1 000,5", french), Some(q(2001, 2)));
        assert_eq!(num("1\u{202f}000", french), Some(q(1000, 1)));
        assert_eq!(num("1 000", EN), None);
    }

    #[test]
    fn radix_and_exponent_literals() {
        assert_eq!(num("0xff", EN), Some(q(255, 1)));
        assert_eq!(num("0b101", EN), Some(q(5, 1)));
        assert_eq!(num("0o17", EN), Some(q(15, 1)));
        assert_eq!(num("1e3", EN), Some(q(1000, 1)));
        assert_eq!(num("1.5e-3", EN), Some(q(3, 2000)));
        assert_eq!(num(".5", EN), Some(q(1, 2)));
    }

    #[test]
    fn a_trailing_e_is_the_constant_not_an_exponent() {
        assert_eq!(
            tokens("2e", EN),
            Some(vec![Token::Num(q(2, 1)), Token::Ident("e".into())])
        );
    }

    #[test]
    fn typographic_operators_and_argument_separators() {
        assert_eq!(
            tokens("5 × 3 ÷ 2 − 1 ** 2", EN),
            Some(vec![
                Token::Num(q(5, 1)),
                Token::Times,
                Token::Num(q(3, 1)),
                Token::Divide,
                Token::Num(q(2, 1)),
                Token::Minus,
                Token::Num(q(1, 1)),
                Token::Power,
                Token::Num(q(2, 1)),
            ])
        );
        assert_eq!(
            tokens("max(1; 2)", DE).map(|t| t.contains(&Token::Comma)),
            Some(true)
        );
        assert_eq!(tokens("2 @ 3", EN), None);
    }
}
