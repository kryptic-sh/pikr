//! A value with its units: a number in base units, what it measures, and
//! the unit it is shown in.

use std::cmp::Ordering;

use num_rational::BigRational;
use num_traits::{Pow, ToPrimitive, Zero};

use super::number::{FloatFn, Num};
use super::units::{Dims, NONE, Unit};

/// The most a unit may be raised to (`m^3`): beyond it dimensions are
/// nonsense and would overflow their counters.
const MAX_UNIT_POWER: i32 = 12;

/// The unit a value is shown in.
#[derive(Clone, Debug)]
pub struct Label {
    /// As written (`km`, `km/h`, `°F`).
    pub text: String,
    /// Its size in base units.
    pub factor: BigRational,
}

#[derive(Clone, Debug)]
pub struct Value {
    /// The amount, in base units (`5 km` is 5000).
    pub num: Num,
    pub dims: Dims,
    /// The unit to show it in; `None` shows base units.
    pub label: Option<Label>,
    /// For a temperature reading (`20 °C`): kelvin at its scale's zero, so
    /// a conversion to another scale can shift it. A value with units
    /// multiplied in is no reading.
    pub zero: Option<BigRational>,
}

impl Value {
    pub fn plain(num: Num) -> Self {
        Self {
            num,
            dims: NONE,
            label: None,
            zero: None,
        }
    }

    /// One of `unit`, named `name`.
    pub fn unit(name: &str, unit: Unit) -> Self {
        Self {
            num: Num::Exact(unit.factor.clone()),
            dims: unit.dims,
            label: Some(Label {
                text: name.to_owned(),
                factor: unit.factor,
            }),
            zero: unit.zero,
        }
    }

    pub fn is_plain(&self) -> bool {
        self.dims == NONE
    }

    /// The number, when the value has no units.
    pub fn number(&self) -> Option<&Num> {
        self.is_plain().then_some(&self.num)
    }

    /// `+` and `-` need the same dimensions; the left side's unit wins.
    fn additive(&self, rhs: &Self, num: Option<Num>) -> Option<Self> {
        if self.dims != rhs.dims {
            return None;
        }
        let label = self.label.clone().or_else(|| rhs.label.clone());
        let zero = self.zero.clone().or_else(|| rhs.zero.clone());
        Some(quantity(num?, self.dims, label, zero))
    }

    pub fn add(&self, rhs: &Self) -> Option<Self> {
        self.additive(rhs, self.num.add(&rhs.num))
    }

    pub fn sub(&self, rhs: &Self) -> Option<Self> {
        self.additive(rhs, self.num.sub(&rhs.num))
    }

    pub fn mul(&self, rhs: &Self) -> Option<Self> {
        let num = self.num.mul(&rhs.num)?;
        if rhs.is_plain() {
            return Some(quantity(
                num,
                self.dims,
                self.label.clone(),
                self.zero.clone(),
            ));
        }
        if self.is_plain() {
            return Some(quantity(num, rhs.dims, rhs.label.clone(), rhs.zero.clone()));
        }
        let dims = combine(&self.dims, &rhs.dims, i8::checked_add)?;
        let label = compose(self.label.as_ref(), rhs.label.as_ref(), '·');
        Some(quantity(num, dims, label, None))
    }

    pub fn div(&self, rhs: &Self) -> Option<Self> {
        let num = self.num.div(&rhs.num)?;
        if rhs.is_plain() {
            return Some(quantity(
                num,
                self.dims,
                self.label.clone(),
                self.zero.clone(),
            ));
        }
        let dims = combine(&self.dims, &rhs.dims, i8::checked_sub)?;
        let label = if self.is_plain() {
            rhs.label.as_ref().map(|r| Label {
                text: format!("1/{}", grouped(&r.text)),
                factor: r.factor.recip(),
            })
        } else {
            compose(self.label.as_ref(), rhs.label.as_ref(), '/')
        };
        Some(quantity(num, dims, label, None))
    }

    pub fn rem(&self, rhs: &Self) -> Option<Self> {
        if self.dims != rhs.dims {
            return None;
        }
        Some(quantity(
            self.num.rem(&rhs.num)?,
            self.dims,
            self.label.clone(),
            self.zero.clone(),
        ))
    }

    pub fn neg(&self) -> Self {
        Self {
            num: self.num.neg(),
            ..self.clone()
        }
    }

    pub fn abs(&self) -> Self {
        Self {
            num: self.num.abs(),
            ..self.clone()
        }
    }

    /// A unit can be raised only to a whole power (`m^2`).
    pub fn pow(&self, exponent: &Self) -> Option<Self> {
        let exponent = exponent.number()?;
        if self.is_plain() {
            return Some(Self::plain(self.num.pow(exponent)?));
        }
        let n = exponent
            .as_exact()
            .filter(|r| r.is_integer())?
            .to_integer()
            .to_i32()
            .filter(|n| *n != 0 && n.abs() <= MAX_UNIT_POWER)?;
        let power = i8::try_from(n).ok()?;
        let dims = combine(&self.dims, &NONE, |d, _| d.checked_mul(power))?;
        let label = self.label.as_ref().map(|l| Label {
            text: format!("{}^{n}", grouped(&l.text)),
            factor: Pow::pow(&l.factor, n),
        });
        Some(quantity(self.num.pow(exponent)?, dims, label, None))
    }

    /// The `q`-th root; a unit's dimensions must divide by `q` (`√(m^2)`).
    pub fn root(&self, q: u32) -> Option<Self> {
        if self.is_plain() {
            return Some(Self::plain(self.num.root(q)?));
        }
        let q = i8::try_from(q).ok()?;
        if self.dims.iter().any(|d| d % q != 0) {
            return None;
        }
        let dims = combine(&self.dims, &NONE, |d, _| Some(d / q))?;
        Some(quantity(
            self.num.root(q.unsigned_abs().into())?,
            dims,
            None,
            None,
        ))
    }

    pub fn factorial(&self) -> Option<Self> {
        Some(Self::plain(self.number()?.factorial()?))
    }

    /// A function of a plain number (`sin`, `ln`).
    pub fn apply(&self, f: FloatFn) -> Option<Self> {
        Some(Self::plain(self.number()?.apply(f)?))
    }

    pub fn log(&self, base: &Self) -> Option<Self> {
        Some(Self::plain(self.number()?.log(base.number()?)?))
    }

    /// Rounds to a whole number of the unit it is shown in (`floor(5.3 km)`
    /// is 5 km).
    pub fn whole_with(&self, f: fn(&BigRational) -> BigRational) -> Option<Self> {
        let Some(label) = &self.label else {
            return Some(Self {
                num: self.num.whole_with(f)?,
                ..self.clone()
            });
        };
        let factor = Num::Exact(label.factor.clone());
        let shown = self.num.div(&factor)?.whole_with(f)?;
        Some(Self {
            num: shown.mul(&factor)?,
            ..self.clone()
        })
    }

    pub fn cmp(&self, other: &Self) -> Option<Ordering> {
        (self.dims == other.dims).then(|| self.num.cmp(&other.num))?
    }

    /// `self` in `target`'s unit (`5 km to mi`), shifting a temperature
    /// reading between scales. `target` must be exactly one unit (`mi`, not
    /// `2 mi`) of the same dimensions.
    pub fn convert(&self, target: &Self) -> Option<Self> {
        if self.is_plain() || self.dims != target.dims {
            return None;
        }
        let label = target.label.clone()?;
        if target.num.as_exact() != Some(&label.factor) {
            return None;
        }
        let num = match (&self.zero, &target.zero) {
            (Some(from), Some(to)) => self.num.add(&Num::Exact(from - to))?,
            _ => self.num.clone(),
        };
        Some(Self {
            num,
            dims: self.dims,
            label: Some(label),
            zero: target.zero.clone(),
        })
    }

    /// The number as shown in its unit (`5 km` is 5), and that unit's name,
    /// or base units' when it has no unit of its own.
    pub fn shown(&self) -> Option<(Num, Option<String>)> {
        if self.is_plain() {
            return Some((self.num.clone(), None));
        }
        match &self.label {
            Some(label) => {
                if label.factor.is_zero() {
                    return None;
                }
                let num = self.num.div(&Num::Exact(label.factor.clone()))?;
                Some((num, Some(label.text.clone())))
            }
            None => Some((self.num.clone(), super::units::base_label(&self.dims))),
        }
    }
}

/// A value of `dims`; one whose units cancelled out (`km/m`) is a plain
/// number.
fn quantity(num: Num, dims: Dims, label: Option<Label>, zero: Option<BigRational>) -> Value {
    if dims == NONE {
        return Value::plain(num);
    }
    Value {
        num,
        dims,
        label,
        zero,
    }
}

/// Each dimension of `a` and `b` combined by `op`, or `None` on overflow.
fn combine(a: &Dims, b: &Dims, op: impl Fn(i8, i8) -> Option<i8>) -> Option<Dims> {
    let mut out = NONE;
    for i in 0..out.len() {
        out[i] = op(a[i], b[i])?;
    }
    Some(out)
}

/// The label for `left op right` when both have one: `km/h`, `N·m`.
fn compose(left: Option<&Label>, right: Option<&Label>, op: char) -> Option<Label> {
    let (left, right) = (left?, right?);
    let factor = if op == '/' {
        &left.factor / &right.factor
    } else {
        &left.factor * &right.factor
    };
    Some(Label {
        text: format!("{}{op}{}", left.text, grouped(&right.text)),
        factor,
    })
}

/// `text` bracketed when it is compound, so it can follow `/` or `^`.
fn grouped(text: &str) -> String {
    if text.contains(['/', '·', '^']) {
        format!("({text})")
    } else {
        text.to_owned()
    }
}
