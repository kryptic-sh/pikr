//! The units calc knows: what each one measures and how big it is in SI
//! base units, exactly, from its definition (an inch is 0.0254 m, a pound
//! 0.45359237 kg).

use num_rational::BigRational;
use num_traits::Zero;

use super::number::decimal;

/// How many dimensions a quantity has: see [`BASE_SYMBOLS`].
pub const DIMENSIONS: usize = 6;

/// The power of each dimension in a quantity: length, mass, time,
/// temperature, data, money.
pub type Dims = [i8; DIMENSIONS];

/// The unit each dimension is measured in internally, and shown in when a
/// result has no unit of its own (`3 m * 4 m` is `12 m^2`).
pub const BASE_SYMBOLS: [&str; DIMENSIONS] = ["m", "kg", "s", "K", "B", "EUR"];

pub const NONE: Dims = [0; DIMENSIONS];

/// A unit: what it measures, its size in base units, and for a temperature
/// scale where its zero sits (how many kelvin a reading of 0 is).
#[derive(Clone, Debug)]
pub struct Unit {
    pub dims: Dims,
    pub factor: BigRational,
    pub zero: Option<BigRational>,
}

struct Def {
    names: &'static [&'static str],
    dims: Dims,
    /// A decimal (`0.0254`, `1e-6`) or a ratio of two (`5/9`).
    factor: &'static str,
    /// Kelvin at a reading of 0, for temperature scales.
    zero: Option<&'static str>,
}

const fn dims(length: i8, mass: i8, time: i8) -> Dims {
    [length, mass, time, 0, 0, 0]
}

const LENGTH: Dims = dims(1, 0, 0);
const AREA: Dims = dims(2, 0, 0);
const VOLUME: Dims = dims(3, 0, 0);
const MASS: Dims = dims(0, 1, 0);
const TIME: Dims = dims(0, 0, 1);
const FREQUENCY: Dims = dims(0, 0, -1);
const SPEED: Dims = dims(1, 0, -1);
const FORCE: Dims = dims(1, 1, -2);
const ENERGY: Dims = dims(2, 1, -2);
const POWER: Dims = dims(2, 1, -3);
const PRESSURE: Dims = dims(-1, 1, -2);
const TEMPERATURE: Dims = [0, 0, 0, 1, 0, 0];
const DATA: Dims = [0, 0, 0, 0, 1, 0];

const fn unit(names: &'static [&'static str], dims: Dims, factor: &'static str) -> Def {
    Def {
        names,
        dims,
        factor,
        zero: None,
    }
}

const fn scale(names: &'static [&'static str], factor: &'static str, zero: &'static str) -> Def {
    Def {
        names,
        dims: TEMPERATURE,
        factor,
        zero: Some(zero),
    }
}

/// Every unit. Short symbols match case-sensitively (`mB` is not `MB`);
/// spelled-out names also match in any case (`Miles`).
const UNITS: &[Def] = &[
    // Length.
    unit(&["m", "meter", "meters", "metre", "metres"], LENGTH, "1"),
    unit(
        &["km", "kilometer", "kilometers", "kilometre", "kilometres"],
        LENGTH,
        "1000",
    ),
    unit(
        &[
            "cm",
            "centimeter",
            "centimeters",
            "centimetre",
            "centimetres",
        ],
        LENGTH,
        "0.01",
    ),
    unit(
        &[
            "mm",
            "millimeter",
            "millimeters",
            "millimetre",
            "millimetres",
        ],
        LENGTH,
        "0.001",
    ),
    unit(
        &["µm", "um", "micrometer", "micrometers", "micron", "microns"],
        LENGTH,
        "1e-6",
    ),
    unit(&["nm", "nanometer", "nanometers"], LENGTH, "1e-9"),
    unit(&["in", "inch", "inches"], LENGTH, "0.0254"),
    unit(&["ft", "foot", "feet"], LENGTH, "0.3048"),
    unit(&["yd", "yard", "yards"], LENGTH, "0.9144"),
    unit(&["mi", "mile", "miles"], LENGTH, "1609.344"),
    unit(&["nmi"], LENGTH, "1852"),
    unit(&["au"], LENGTH, "149597870700"),
    unit(
        &["ly", "lightyear", "lightyears"],
        LENGTH,
        "9460730472580800",
    ),
    // Area.
    unit(&["ha", "hectare", "hectares"], AREA, "10000"),
    unit(&["acre", "acres"], AREA, "4046.8564224"),
    // Volume (US customary measures).
    unit(
        &["L", "l", "liter", "liters", "litre", "litres"],
        VOLUME,
        "0.001",
    ),
    unit(
        &[
            "mL",
            "ml",
            "milliliter",
            "milliliters",
            "millilitre",
            "millilitres",
        ],
        VOLUME,
        "1e-6",
    ),
    unit(&["cL", "cl"], VOLUME, "1e-5"),
    unit(&["dL", "dl"], VOLUME, "1e-4"),
    unit(&["gal", "gallon", "gallons"], VOLUME, "0.003785411784"),
    unit(&["qt", "quart", "quarts"], VOLUME, "0.000946352946"),
    unit(&["pt", "pint", "pints"], VOLUME, "0.000473176473"),
    unit(&["cup", "cups"], VOLUME, "0.0002365882365"),
    unit(&["floz"], VOLUME, "0.0000295735295625"),
    unit(
        &["tbsp", "tablespoon", "tablespoons"],
        VOLUME,
        "0.00001478676478125",
    ),
    unit(
        &["tsp", "teaspoon", "teaspoons"],
        VOLUME,
        "0.00000492892159375",
    ),
    // Mass.
    unit(&["kg", "kilogram", "kilograms"], MASS, "1"),
    unit(&["g", "gram", "grams"], MASS, "0.001"),
    unit(&["mg", "milligram", "milligrams"], MASS, "1e-6"),
    unit(&["µg", "ug", "microgram", "micrograms"], MASS, "1e-9"),
    unit(&["t", "tonne", "tonnes"], MASS, "1000"),
    unit(&["lb", "lbs", "pound", "pounds"], MASS, "0.45359237"),
    unit(&["oz", "ounce", "ounces"], MASS, "0.028349523125"),
    unit(&["st", "stone", "stones"], MASS, "6.35029318"),
    // Time (a year is the Julian year, 365.25 days).
    unit(&["s", "sec", "secs", "second", "seconds"], TIME, "1"),
    unit(&["ms", "millisecond", "milliseconds"], TIME, "0.001"),
    unit(&["µs", "us", "microsecond", "microseconds"], TIME, "1e-6"),
    unit(&["ns", "nanosecond", "nanoseconds"], TIME, "1e-9"),
    unit(&["min", "mins", "minute", "minutes"], TIME, "60"),
    unit(&["h", "hr", "hrs", "hour", "hours"], TIME, "3600"),
    unit(&["d", "day", "days"], TIME, "86400"),
    unit(&["wk", "week", "weeks"], TIME, "604800"),
    unit(&["yr", "year", "years"], TIME, "31557600"),
    // Frequency.
    unit(&["Hz", "hertz"], FREQUENCY, "1"),
    unit(&["kHz"], FREQUENCY, "1000"),
    unit(&["MHz"], FREQUENCY, "1e6"),
    unit(&["GHz"], FREQUENCY, "1e9"),
    // Speed.
    unit(&["mph"], SPEED, "0.44704"),
    unit(&["kph"], SPEED, "1000/3600"),
    unit(&["kn", "knot", "knots"], SPEED, "1852/3600"),
    // Force.
    unit(&["N", "newton", "newtons"], FORCE, "1"),
    unit(&["kN"], FORCE, "1000"),
    unit(&["lbf"], FORCE, "4.4482216152605"),
    // Energy.
    unit(&["J", "joule", "joules"], ENERGY, "1"),
    unit(&["kJ"], ENERGY, "1000"),
    unit(&["MJ"], ENERGY, "1e6"),
    unit(&["cal", "calorie", "calories"], ENERGY, "4.184"),
    unit(&["kcal", "Cal"], ENERGY, "4184"),
    unit(&["Wh"], ENERGY, "3600"),
    unit(&["kWh"], ENERGY, "3600000"),
    unit(&["eV"], ENERGY, "1.602176634e-19"),
    unit(&["BTU", "btu"], ENERGY, "1055.05585262"),
    // Power.
    unit(&["W", "watt", "watts"], POWER, "1"),
    unit(&["kW"], POWER, "1000"),
    unit(&["MW"], POWER, "1e6"),
    unit(&["GW"], POWER, "1e9"),
    unit(&["hp", "horsepower"], POWER, "745.69987158227022"),
    // Pressure.
    unit(&["Pa", "pascal", "pascals"], PRESSURE, "1"),
    unit(&["hPa"], PRESSURE, "100"),
    unit(&["kPa"], PRESSURE, "1000"),
    unit(&["MPa"], PRESSURE, "1e6"),
    unit(&["bar"], PRESSURE, "100000"),
    unit(&["mbar"], PRESSURE, "100"),
    unit(&["atm"], PRESSURE, "101325"),
    // A pound-force on a square inch.
    unit(&["psi"], PRESSURE, "4.4482216152605/0.00064516"),
    unit(&["mmHg"], PRESSURE, "133.322387415"),
    // Data.
    unit(&["B", "byte", "bytes"], DATA, "1"),
    unit(&["bit", "bits"], DATA, "1/8"),
    unit(&["kB", "KB"], DATA, "1000"),
    unit(&["MB"], DATA, "1e6"),
    unit(&["GB"], DATA, "1e9"),
    unit(&["TB"], DATA, "1e12"),
    unit(&["PB"], DATA, "1e15"),
    unit(&["KiB"], DATA, "1024"),
    unit(&["MiB"], DATA, "1048576"),
    unit(&["GiB"], DATA, "1073741824"),
    unit(&["TiB"], DATA, "1099511627776"),
    unit(&["kbit", "Kbit"], DATA, "125"),
    unit(&["Mbit"], DATA, "125000"),
    unit(&["Gbit"], DATA, "125000000"),
    // Temperature scales: kelvin per degree, and kelvin at 0 degrees.
    scale(&["K", "kelvin"], "1", "0"),
    scale(&["°C", "degC", "celsius"], "1", "273.15"),
    scale(&["°F", "degF", "fahrenheit"], "5/9", "45967/180"),
];

/// Names shorter than this are symbols and match only in their own case.
const SPELLED_OUT: usize = 4;

/// The unit named `name`, if calc knows one.
pub fn lookup(name: &str) -> Option<Unit> {
    let def = UNITS
        .iter()
        .find(|def| def.names.contains(&name))
        .or_else(|| {
            (name.chars().count() >= SPELLED_OUT).then(|| {
                UNITS.iter().find(|def| {
                    def.names
                        .iter()
                        .any(|n| n.chars().count() >= SPELLED_OUT && n.eq_ignore_ascii_case(name))
                })
            })?
        })?;
    Some(Unit {
        dims: def.dims,
        factor: rational(def.factor)?,
        zero: def.zero.and_then(rational),
    })
}

/// A table factor: a decimal, or the ratio of two.
fn rational(text: &str) -> Option<BigRational> {
    let one = |part: &str| -> Option<BigRational> {
        let (mantissa, exponent) = part.split_once('e').unwrap_or((part, "0"));
        let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
        decimal(whole, fraction, exponent.parse().ok()?)
    };
    match text.split_once('/') {
        Some((n, d)) => {
            let d = one(d)?;
            (!d.is_zero()).then(|| one(n).map(|n| n / d))?
        }
        None => one(text),
    }
}

/// The name of `dims` in base units (`m/s`, `kg·m^2/s^2`), or `None` when
/// it has none (a plain number).
pub fn base_label(dims: &Dims) -> Option<String> {
    let part = |i: usize, power: i8| match power {
        1 => BASE_SYMBOLS[i].to_owned(),
        p => format!("{}^{p}", BASE_SYMBOLS[i]),
    };
    let above: Vec<String> = (0..DIMENSIONS)
        .filter(|&i| dims[i] > 0)
        .map(|i| part(i, dims[i]))
        .collect();
    let below: Vec<String> = (0..DIMENSIONS)
        .filter(|&i| dims[i] < 0)
        .map(|i| part(i, -dims[i]))
        .collect();
    if above.is_empty() && below.is_empty() {
        return None;
    }
    let top = if above.is_empty() {
        "1".to_owned()
    } else {
        above.join("·")
    };
    Some(match below.len() {
        0 => top,
        1 => format!("{top}/{}", below[0]),
        _ => format!("{top}/({})", below.join("·")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_unit_parses_and_no_name_is_listed_twice() {
        let mut seen = std::collections::HashSet::new();
        for def in UNITS {
            assert!(rational(def.factor).is_some(), "{:?}", def.names);
            if let Some(zero) = def.zero {
                assert!(rational(zero).is_some(), "{:?}", def.names);
            }
            for name in def.names {
                assert!(seen.insert(*name), "{name} is listed twice");
            }
        }
    }

    #[test]
    fn symbols_keep_their_case_and_words_do_not() {
        assert!(lookup("MB").is_some());
        assert!(lookup("mB").is_none());
        assert!(lookup("Miles").is_some());
        assert_eq!(lookup("MILE").map(|u| u.dims), Some(LENGTH));
        assert!(lookup("M").is_none());
    }

    #[test]
    fn factors_are_exact() {
        let psi = lookup("psi").unwrap().factor;
        assert_eq!(psi, rational("44482216152605/6451600000").unwrap());
        // 459.67 °F below zero is 0 K, at 5/9 K per degree.
        assert_eq!(
            lookup("°F").unwrap().zero,
            rational("459.67").map(|z| z * rational("5/9").unwrap())
        );
    }

    #[test]
    fn base_labels_name_derived_units() {
        assert_eq!(base_label(&SPEED).as_deref(), Some("m/s"));
        assert_eq!(base_label(&ENERGY).as_deref(), Some("m^2·kg/s^2"));
        assert_eq!(base_label(&FREQUENCY).as_deref(), Some("1/s"));
        assert_eq!(base_label(&NONE), None);
    }
}
