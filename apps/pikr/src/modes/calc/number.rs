//! Calc's numbers: exact rationals wherever the maths allows, and
//! arbitrary-precision floats for what it does not (π, e, roots that are
//! not perfect, logarithms, trigonometry).

use std::cell::RefCell;
use std::cmp::Ordering;

use astro_float::{BigFloat, Consts, Radix, RoundingMode};
use num_bigint::BigInt;
use num_rational::BigRational;
use num_traits::{One, Pow, Signed, ToPrimitive, Zero};

/// Bits of precision for approximate values: about 57 significant decimal
/// digits, far past the 20 a result is shown with, so rounding error never
/// reaches the shown digits.
const PRECISION: usize = 192;

const ROUNDING: RoundingMode = RoundingMode::ToEven;

/// The largest `n` whose factorial calc computes. 3000! has 9131 digits;
/// past this the work per keystroke stops being negligible.
const MAX_FACTORIAL: u32 = 3000;

/// The most bits an exact power may produce (about 30,000 decimal digits).
/// A larger one is computed approximately instead.
const MAX_EXACT_BITS: u64 = 100_000;

/// The largest root index tried exactly (`x^(1/q)` for `q` up to this).
const MAX_EXACT_ROOT: u32 = 64;

/// The largest power of ten a literal's exponent may name (`1e9999`).
const MAX_LITERAL_EXPONENT: i64 = 10_000;

/// The largest power of ten an approximate value is read back into a
/// rational at, for rounding and display. It matches [`MAX_EXACT_BITS`]'s
/// size; past it a result (`2^1000000`) is too large to write out and is no
/// result.
const MAX_VALUE_EXPONENT: i64 = 30_103;

/// A value during evaluation.
#[derive(Clone, Debug)]
pub enum Num {
    /// Known exactly.
    Exact(BigRational),
    /// Known to [`PRECISION`] bits; always finite.
    Approx(BigFloat),
}

thread_local! {
    /// astro-float's constant cache (π, e, ln 2, ln 10), kept between
    /// evaluations so each keystroke does not recompute them.
    static CONSTS: RefCell<Option<Consts>> = const { RefCell::new(None) };
}

/// Runs `f` with the constant cache, or `None` when it cannot be made.
fn with_consts<T>(f: impl FnOnce(&mut Consts) -> T) -> Option<T> {
    CONSTS.with(|cell| {
        let mut slot = cell.borrow_mut();
        if slot.is_none() {
            *slot = Consts::new().ok();
        }
        slot.as_mut().map(f)
    })
}

/// A float function in astro-float's shape, such as `BigFloat::sin`.
pub type FloatFn = fn(&BigFloat, usize, RoundingMode, &mut Consts) -> BigFloat;

impl Num {
    pub fn int(i: i64) -> Self {
        Self::Exact(BigRational::from_integer(BigInt::from(i)))
    }

    pub fn pi() -> Option<Self> {
        Self::approx(with_consts(|cc| cc.pi(PRECISION, ROUNDING))?)
    }

    pub fn e() -> Option<Self> {
        Self::approx(with_consts(|cc| cc.e(PRECISION, ROUNDING))?)
    }

    /// An approximate value, or `None` when it is NaN or infinite.
    fn approx(f: BigFloat) -> Option<Self> {
        (!f.is_nan() && !f.is_inf()).then_some(Self::Approx(f))
    }

    /// The value as a float: exact when it is one, else rounded.
    fn to_float(&self) -> Option<BigFloat> {
        match self {
            Self::Approx(f) => Some(f.clone()),
            Self::Exact(r) => with_consts(|cc| {
                let parse = |n: &BigInt, cc: &mut Consts| {
                    BigFloat::parse(&n.to_string(), Radix::Dec, PRECISION, ROUNDING, cc)
                };
                let n = parse(r.numer(), cc);
                let d = parse(r.denom(), cc);
                n.div(&d, PRECISION, ROUNDING)
            }),
        }
    }

    /// The value as a rational: exactly, or the float's own value.
    pub fn to_rational(&self) -> Option<BigRational> {
        match self {
            Self::Exact(r) => Some(r.clone()),
            Self::Approx(f) => float_to_rational(f),
        }
    }

    pub fn as_exact(&self) -> Option<&BigRational> {
        match self {
            Self::Exact(r) => Some(r),
            Self::Approx(_) => None,
        }
    }

    pub fn is_zero(&self) -> bool {
        match self {
            Self::Exact(r) => r.is_zero(),
            Self::Approx(f) => f.is_zero(),
        }
    }

    pub fn add(&self, rhs: &Self) -> Option<Self> {
        match (self, rhs) {
            (Self::Exact(a), Self::Exact(b)) => Some(Self::Exact(a + b)),
            _ => Self::approx(self.to_float()?.add(&rhs.to_float()?, PRECISION, ROUNDING)),
        }
    }

    pub fn sub(&self, rhs: &Self) -> Option<Self> {
        match (self, rhs) {
            (Self::Exact(a), Self::Exact(b)) => Some(Self::Exact(a - b)),
            _ => Self::approx(self.to_float()?.sub(&rhs.to_float()?, PRECISION, ROUNDING)),
        }
    }

    pub fn mul(&self, rhs: &Self) -> Option<Self> {
        match (self, rhs) {
            (Self::Exact(a), Self::Exact(b)) => Some(Self::Exact(a * b)),
            _ => Self::approx(self.to_float()?.mul(&rhs.to_float()?, PRECISION, ROUNDING)),
        }
    }

    /// Dividing by zero is no result.
    pub fn div(&self, rhs: &Self) -> Option<Self> {
        if rhs.is_zero() {
            return None;
        }
        match (self, rhs) {
            (Self::Exact(a), Self::Exact(b)) => Some(Self::Exact(a / b)),
            _ => Self::approx(self.to_float()?.div(&rhs.to_float()?, PRECISION, ROUNDING)),
        }
    }

    /// The remainder, with the sign of the dividend (`-7 % 3` is -1).
    pub fn rem(&self, rhs: &Self) -> Option<Self> {
        if rhs.is_zero() {
            return None;
        }
        match (self, rhs) {
            (Self::Exact(a), Self::Exact(b)) => Some(Self::Exact(a - b * (a / b).trunc())),
            _ => Self::approx(self.to_float()?.rem(&rhs.to_float()?)),
        }
    }

    pub fn neg(&self) -> Self {
        match self {
            Self::Exact(r) => Self::Exact(-r),
            Self::Approx(f) => Self::Approx(f.neg()),
        }
    }

    pub fn abs(&self) -> Self {
        match self {
            Self::Exact(r) => Self::Exact(r.abs()),
            Self::Approx(f) => Self::Approx(f.abs()),
        }
    }

    /// `self ^ exponent`: exact for a whole exponent and for a root that
    /// comes out rational (`8^(1/3)`), approximate otherwise.
    pub fn pow(&self, exponent: &Self) -> Option<Self> {
        if let (Self::Exact(base), Self::Exact(exp)) = (self, exponent) {
            if base.is_zero() && exp.is_negative() {
                return None;
            }
            if let Some(value) = exact_pow(base, exp) {
                return Some(Self::Exact(value));
            }
        }
        // A negative base with a fractional exponent is NaN: no result.
        let (base, exp) = (self.to_float()?, exponent.to_float()?);
        Self::approx(with_consts(|cc| base.pow(&exp, PRECISION, ROUNDING, cc))?)
    }

    /// `n!` for a whole `n ≥ 0`, exactly.
    pub fn factorial(&self) -> Option<Self> {
        let n = whole(self.as_exact()?)?.to_u32()?;
        if n > MAX_FACTORIAL {
            return None;
        }
        let product = (2..=n).fold(BigInt::one(), |acc, k| acc * k);
        Some(Self::Exact(BigRational::from_integer(product)))
    }

    /// The `q`-th root: exact when it is rational, else approximate. An even
    /// root of a negative number is no result.
    pub fn root(&self, q: u32) -> Option<Self> {
        if let Self::Exact(r) = self
            && let Some(root) = exact_root(r, q)
        {
            return Some(Self::Exact(root));
        }
        let f = self.to_float()?;
        match q {
            2 if !f.is_negative() => Self::approx(f.sqrt(PRECISION, ROUNDING)),
            3 => Self::approx(f.cbrt(PRECISION, ROUNDING)),
            _ => None,
        }
    }

    /// A float function of the value, such as `sin` or `ln`.
    pub fn apply(&self, f: FloatFn) -> Option<Self> {
        let x = self.to_float()?;
        Self::approx(with_consts(|cc| f(&x, PRECISION, ROUNDING, cc))?)
    }

    /// Rounds to a whole number with `f` (`floor`, `ceil`, `round`,
    /// `trunc`): exact, even for an approximate value.
    pub fn whole_with(&self, f: fn(&BigRational) -> BigRational) -> Option<Self> {
        Some(Self::Exact(f(&self.to_rational()?)))
    }

    pub fn cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.to_rational()?.cmp(&other.to_rational()?))
    }

    /// `log_base(self)`: exact when it is a whole number (`log(1000)` is 3).
    pub fn log(&self, base: &Self) -> Option<Self> {
        let (x, b) = (self.to_float()?, base.to_float()?);
        let value = Self::approx(with_consts(|cc| {
            x.ln(PRECISION, ROUNDING, cc)
                .div(&b.ln(PRECISION, ROUNDING, cc), PRECISION, ROUNDING)
        })?)?;
        if let (Self::Exact(x), Self::Exact(b)) = (self, base)
            && let Some(k) = value
                .to_rational()
                .and_then(|r| r.round().to_integer().to_i32())
            && exact_pow(b, &BigRational::from_integer(k.into())).as_ref() == Some(x)
        {
            return Some(Self::int(k.into()));
        }
        Some(value)
    }
}

/// `r` as a whole number, if it is one.
fn whole(r: &BigRational) -> Option<BigInt> {
    r.is_integer().then(|| r.to_integer())
}

/// `base ^ exp` when it is rational and small enough to compute.
fn exact_pow(base: &BigRational, exp: &BigRational) -> Option<BigRational> {
    let q = exp.denom().to_u32()?;
    let p = exp.numer().to_i32()?;
    let base = if q == 1 {
        base.clone()
    } else {
        exact_root(base, q)?
    };
    if base.is_zero() {
        return (p >= 0).then(|| if p == 0 { BigRational::one() } else { base });
    }
    let bits = base.numer().bits().max(base.denom().bits());
    if bits.saturating_mul(u64::from(p.unsigned_abs())) > MAX_EXACT_BITS {
        return None;
    }
    Some(Pow::pow(&base, p))
}

/// The rational `q`-th root of `r`, when there is one.
fn exact_root(r: &BigRational, q: u32) -> Option<BigRational> {
    if q == 0 || q > MAX_EXACT_ROOT || (r.is_negative() && q.is_multiple_of(2)) {
        return None;
    }
    let root = |n: &BigInt| -> Option<BigInt> {
        let candidate = n.nth_root(q);
        (Pow::pow(&candidate, q) == *n).then_some(candidate)
    };
    Some(BigRational::new(root(r.numer())?, root(r.denom())?))
}

/// A float's exact value, read back through its decimal digits
/// (`1.4142…e+0`).
fn float_to_rational(f: &BigFloat) -> Option<BigRational> {
    let text = with_consts(|cc| f.format(Radix::Dec, ROUNDING, cc).ok())??;
    let (mantissa, exponent) = text.split_once('e').unwrap_or((&text, "0"));
    let (negative, mantissa) = mantissa
        .strip_prefix('-')
        .map_or((false, mantissa), |m| (true, m));
    let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let value = scaled(whole, fraction, exponent.parse().ok()?, MAX_VALUE_EXPONENT)?;
    Some(if negative { -value } else { value })
}

/// The rational a literal `whole.fraction × 10^exponent` names, from ASCII
/// digits, or `None` past [`MAX_LITERAL_EXPONENT`].
pub fn decimal(whole: &str, fraction: &str, exponent: i64) -> Option<BigRational> {
    scaled(whole, fraction, exponent, MAX_LITERAL_EXPONENT)
}

/// `whole.fraction × 10^exponent`, or `None` when `|exponent|` passes `max`.
fn scaled(whole: &str, fraction: &str, exponent: i64, max: i64) -> Option<BigRational> {
    if exponent.abs() > max {
        return None;
    }
    let digits = format!("{whole}{fraction}");
    let mantissa: BigInt = if digits.is_empty() {
        BigInt::zero()
    } else {
        digits.parse().ok()?
    };
    let scale = exponent - i64::try_from(fraction.len()).ok()?;
    let power: BigInt = Pow::pow(&BigInt::from(10), scale.unsigned_abs());
    Some(if scale >= 0 {
        BigRational::from_integer(mantissa * power)
    } else {
        BigRational::new(mantissa, power)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exact(n: i64, d: i64) -> Num {
        Num::Exact(BigRational::new(n.into(), d.into()))
    }

    fn rational(n: &Num) -> BigRational {
        n.as_exact().cloned().expect("exact")
    }

    fn r(n: i64, d: i64) -> BigRational {
        BigRational::new(n.into(), d.into())
    }

    #[test]
    fn arithmetic_on_exact_values_stays_exact() {
        assert_eq!(rational(&exact(1, 10).add(&exact(1, 5)).unwrap()), r(3, 10));
        assert_eq!(rational(&exact(1, 3).mul(&Num::int(3)).unwrap()), r(1, 1));
        assert_eq!(rational(&Num::int(1).div(&Num::int(3)).unwrap()), r(1, 3));
        assert!(Num::int(1).div(&Num::int(0)).is_none());
        assert_eq!(rational(&Num::int(-7).rem(&Num::int(3)).unwrap()), r(-1, 1));
    }

    #[test]
    fn powers_and_roots_are_exact_when_they_can_be() {
        assert_eq!(
            rational(&Num::int(2).pow(&Num::int(100)).unwrap()).to_string(),
            "1267650600228229401496703205376"
        );
        assert_eq!(rational(&Num::int(2).pow(&Num::int(-2)).unwrap()), r(1, 4));
        assert_eq!(rational(&Num::int(8).pow(&exact(1, 3)).unwrap()), r(2, 1));
        assert_eq!(rational(&Num::int(-8).root(3).unwrap()), r(-2, 1));
        assert_eq!(rational(&exact(9, 4).root(2).unwrap()), r(3, 2));
        assert!(matches!(Num::int(2).root(2), Some(Num::Approx(_))));
        assert!(Num::int(-4).root(2).is_none());
        assert!(Num::int(0).pow(&Num::int(-1)).is_none());
        assert_eq!(rational(&Num::int(0).pow(&Num::int(0)).unwrap()), r(1, 1));
        assert!(Num::int(-8).pow(&exact(1, 2)).is_none());
    }

    #[test]
    fn a_huge_power_falls_back_to_approximate() {
        let big = Num::int(10).pow(&Num::int(1_000_000)).unwrap();
        assert!(matches!(big, Num::Approx(_)));
    }

    #[test]
    fn factorial_is_exact_and_bounded() {
        assert_eq!(
            rational(&Num::int(25).factorial().unwrap()).to_string(),
            "15511210043330985984000000"
        );
        assert!(Num::int(-1).factorial().is_none());
        assert!(exact(5, 2).factorial().is_none());
        assert!(Num::int(i64::from(MAX_FACTORIAL) + 1).factorial().is_none());
    }

    #[test]
    fn a_whole_logarithm_is_exact() {
        assert_eq!(
            rational(&Num::int(1000).log(&Num::int(10)).unwrap()),
            r(3, 1)
        );
        assert_eq!(rational(&exact(1, 8).log(&Num::int(2)).unwrap()), r(-3, 1));
        assert!(matches!(
            Num::int(1001).log(&Num::int(10)),
            Some(Num::Approx(_))
        ));
        assert!(Num::int(-1).log(&Num::int(10)).is_none());
    }

    #[test]
    fn an_approximate_value_reads_back_as_its_digits() {
        let root2 = Num::int(2).root(2).unwrap().to_rational().unwrap();
        let squared = &root2 * &root2;
        let error = (squared - r(2, 1)).abs();
        assert!(error < r(1, 1) / BigRational::from_integer(BigInt::from(10).pow(50u32)));
    }

    #[test]
    fn decimals_read_exactly() {
        assert_eq!(decimal("0", "1", 0), Some(r(1, 10)));
        assert_eq!(decimal("15", "", -4), Some(r(15, 10000)));
        assert_eq!(decimal("1", "", 3), Some(r(1000, 1)));
        assert_eq!(decimal("1", "", MAX_LITERAL_EXPONENT + 1), None);
    }
}
