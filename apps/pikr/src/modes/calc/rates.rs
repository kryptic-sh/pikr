//! Exchange rates: read from a cache on disk at once, refreshed from the
//! network on a background thread so calc never waits for it.
//!
//! Fiat rates are the European Central Bank's daily reference rates
//! ([`ECB_URL`]), quoted against the euro. They are fetched with the system's
//! `curl`, so pikr carries no TLS stack of its own; without `curl`, or
//! offline, the cached rates stay in use and a first run has none.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use num_rational::BigRational;
use num_traits::{Signed, Zero};
use serde::{Deserialize, Serialize};

use super::number::decimal;

/// The ECB's reference rates for the latest working day, in EUR.
const ECB_URL: &str = "https://www.ecb.europa.eu/stats/eurofxref/eurofxref-daily.xml";

/// The cache file for the ECB's rates, in pikr's state directory.
const ECB_CACHE: &str = "rates-ecb.toml";

/// How old the cached ECB rates may get before a refresh. The ECB publishes
/// once a working day, around 16:00 CET.
const ECB_MAX_AGE: Duration = Duration::from_secs(12 * 60 * 60);

/// How long a fetch may take before `curl` gives up.
const FETCH_TIMEOUT_SECS: &str = "15";

/// The most bytes a rates response is read to; the ECB's is about 2 KB.
const MAX_RESPONSE: u64 = 1024 * 1024;

/// Currencies whose amounts have no minor unit (ISO 4217 exponent 0), shown
/// without decimals. Every other currency is shown with two.
const WHOLE_CURRENCIES: &[&str] = &["JPY", "KRW", "ISK"];

/// Currency symbols and the code each stands for, where the symbol is not
/// shared by several currencies in everyday use.
const SYMBOLS: &[(&str, &str)] = &[
    ("€", "EUR"),
    ("$", "USD"),
    ("£", "GBP"),
    ("¥", "JPY"),
    ("₹", "INR"),
];

/// The rates calc converts money with: euros per unit of each currency.
#[derive(Clone, Debug, Default)]
pub struct Rates {
    per_euro: HashMap<String, BigRational>,
    /// The day the rates are for, as the source dates them (`2026-11-12`).
    date: Option<String>,
}

impl Rates {
    /// The currency named `name` (`USD`, `usd`, `$`), as its code and the
    /// euros one unit of it is worth, if there are rates for it. The euro
    /// needs none.
    pub fn currency(&self, name: &str) -> Option<(String, BigRational)> {
        let code = SYMBOLS
            .iter()
            .find(|(symbol, _)| *symbol == name)
            .map_or_else(|| name.to_ascii_uppercase(), |(_, code)| (*code).to_owned());
        if code.len() != 3 {
            return None;
        }
        if code == "EUR" {
            return Some((code, BigRational::from_integer(1.into())));
        }
        let per_euro = self.per_euro.get(&code)?;
        Some((code, per_euro.recip()))
    }

    /// What the rates' date is, to show beside a converted amount.
    pub fn note(&self) -> Option<String> {
        self.date
            .as_ref()
            .map(|date| format!("ECB reference rates of {date}"))
    }
}

/// Decimals an amount in `code` is shown with.
pub fn minor_units(code: &str) -> u32 {
    if WHOLE_CURRENCIES.contains(&code) {
        0
    } else {
        2
    }
}

/// What a cache file holds.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Cache {
    /// Seconds since the Unix epoch when it was fetched.
    fetched: u64,
    date: String,
    /// Units of each currency per euro, as the source wrote them.
    per_euro: HashMap<String, String>,
}

impl Cache {
    fn rates(&self) -> Rates {
        let per_euro = self
            .per_euro
            .iter()
            .filter_map(|(code, rate)| Some((code.clone(), positive(rate)?)))
            .collect();
        Rates {
            per_euro,
            date: (!self.date.is_empty()).then(|| self.date.clone()),
        }
    }

    fn is_stale(&self, now: SystemTime) -> bool {
        let fetched = UNIX_EPOCH + Duration::from_secs(self.fetched);
        now.duration_since(fetched)
            .map_or(true, |age| age > ECB_MAX_AGE)
    }
}

/// A rate as written (`1.0812`), if it is a positive decimal.
fn positive(text: &str) -> Option<BigRational> {
    let (whole, fraction) = text.trim().split_once('.').unwrap_or((text.trim(), ""));
    if whole.is_empty()
        || !whole
            .bytes()
            .chain(fraction.bytes())
            .all(|b| b.is_ascii_digit())
    {
        return None;
    }
    let value = decimal(whole, fraction, 0)?;
    (value.is_positive() && !value.is_zero()).then_some(value)
}

fn cache_path() -> Option<PathBuf> {
    crate::picker::state_file_path(ECB_CACHE)
}

fn read_cache(path: &Path) -> Option<Cache> {
    toml::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

/// The cached rates, read now; empty when there are none yet.
pub fn load() -> Rates {
    cache_path()
        .and_then(|path| read_cache(&path))
        .map(|cache| cache.rates())
        .unwrap_or_default()
}

/// Starts refreshing the cached rates when they are missing or stale, on a
/// thread of its own. The receiver gets `()` once new rates are on disk, and
/// sees the sender go when the refresh is over either way; `None` when no
/// refresh was needed.
pub fn refresh_in_background() -> Option<mpsc::Receiver<()>> {
    let path = cache_path()?;
    if read_cache(&path).is_some_and(|cache| !cache.is_stale(SystemTime::now())) {
        return None;
    }
    let (done, landed) = mpsc::channel();
    let spawned =
        std::thread::Builder::new()
            .name("pikr-rates".into())
            .spawn(move || match refresh(&path) {
                Ok(()) => {
                    let _ = done.send(());
                }
                Err(e) => tracing::debug!(error = %e, "exchange rates not refreshed"),
            });
    if let Err(e) = spawned {
        tracing::debug!(error = %e, "cannot start the exchange-rate refresh");
        return None;
    }
    Some(landed)
}

/// Fetches the ECB's rates and replaces the cache at `path` with them.
fn refresh(path: &Path) -> anyhow::Result<()> {
    let xml = fetch(ECB_URL)?;
    let (date, per_euro) =
        parse_ecb(&xml).ok_or_else(|| anyhow::anyhow!("the ECB's rates did not parse"))?;
    let fetched = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let cache = Cache {
        fetched,
        date,
        per_euro,
    };
    write_atomically(path, &toml::to_string(&cache)?)
}

/// `url`'s body, fetched with the system's `curl`.
fn fetch(url: &str) -> anyhow::Result<String> {
    use std::io::Read;

    let mut command = Command::new("curl");
    command
        .args(["--fail", "--silent", "--show-error", "--location"])
        .args(["--proto", "=https", "--max-time", FETCH_TIMEOUT_SECS, url])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW: pikr is a windowed program, and a console
        // program it starts would otherwise open a console window.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = command.spawn()?;
    let mut body = String::new();
    child
        .stdout
        .take()
        .ok_or_else(|| anyhow::anyhow!("no stdout from curl"))?
        .take(MAX_RESPONSE)
        .read_to_string(&mut body)?;
    let status = child.wait()?;
    anyhow::ensure!(status.success(), "curl exited with {status}");
    Ok(body)
}

/// Writes `text` to a sibling of `path` and renames it into place, so a
/// reader never sees half a file, even when pikr exits mid-write.
fn write_atomically(path: &Path, text: &str) -> anyhow::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut partial = path.as_os_str().to_owned();
    partial.push(".partial");
    let partial = PathBuf::from(partial);
    std::fs::write(&partial, text)?;
    std::fs::rename(&partial, path)?;
    Ok(())
}

/// The date and the rates in the ECB's daily file: its `<Cube time='…'>`
/// and each `<Cube currency='USD' rate='1.0812'/>`. `None` when it holds no
/// date or no rate.
fn parse_ecb(xml: &str) -> Option<(String, HashMap<String, String>)> {
    let attribute = |tag: &str, name: &str| -> Option<String> {
        let start = tag.find(&format!("{name}="))? + name.len() + 1;
        let quote = tag[start..].chars().next()?;
        let rest = &tag[start + quote.len_utf8()..];
        Some(rest[..rest.find(quote)?].to_owned())
    };
    let mut date = None;
    let mut rates = HashMap::new();
    for tag in xml.split('<').filter(|t| t.starts_with("Cube")) {
        if let Some(time) = attribute(tag, "time") {
            date = Some(time);
        }
        if let (Some(code), Some(rate)) = (attribute(tag, "currency"), attribute(tag, "rate"))
            && code.len() == 3
            && code.bytes().all(|b| b.is_ascii_uppercase())
            && positive(&rate).is_some()
        {
            rates.insert(code, rate);
        }
    }
    (!rates.is_empty()).then_some((date?, rates))
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// The shape of the ECB's file, trimmed to three currencies.
    const ECB_SAMPLE: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>
<gesmes:Envelope xmlns:gesmes=\"http://www.gesmes.org/xml/2002-08-01\" xmlns=\"http://www.ecb.int/vocabulary/2002-08-01/eurofxref\">
\t<gesmes:subject>Reference rates</gesmes:subject>
\t<Cube>
\t\t<Cube time='2026-11-12'>
\t\t\t<Cube currency='USD' rate='1.0812'/>
\t\t\t<Cube currency='JPY' rate='165.43'/>
\t\t\t<Cube currency='GBP' rate='0.8350'/>
\t\t</Cube>
\t</Cube>
</gesmes:Envelope>";

    /// Rates as the sample gives them, for other modules' tests.
    pub fn sample_rates() -> Rates {
        let (date, per_euro) = parse_ecb(ECB_SAMPLE).unwrap();
        Cache {
            fetched: 0,
            date,
            per_euro,
        }
        .rates()
    }

    #[test]
    fn the_ecb_file_parses() {
        let (date, rates) = parse_ecb(ECB_SAMPLE).unwrap();
        assert_eq!(date, "2026-11-12");
        assert_eq!(rates.len(), 3);
        assert_eq!(rates["USD"], "1.0812");
        assert_eq!(parse_ecb("<html>maintenance</html>"), None);
    }

    #[test]
    fn currencies_by_code_symbol_and_case() {
        let rates = sample_rates();
        let usd = BigRational::new(10000.into(), 10812.into());
        assert_eq!(rates.currency("USD"), Some(("USD".into(), usd.clone())));
        assert_eq!(rates.currency("usd"), Some(("USD".into(), usd)));
        assert_eq!(
            rates.currency("$").map(|(code, _)| code).as_deref(),
            Some("USD")
        );
        assert_eq!(
            rates.currency("€").map(|(code, _)| code).as_deref(),
            Some("EUR")
        );
        assert_eq!(rates.currency("CHF"), None);
        assert_eq!(Rates::default().currency("USD"), None);
        assert!(Rates::default().currency("EUR").is_some());
        assert_eq!(
            rates.note().as_deref(),
            Some("ECB reference rates of 2026-11-12")
        );
    }

    #[test]
    fn bad_rates_are_dropped() {
        assert!(positive("1.0812").is_some());
        assert!(positive("0").is_none());
        assert!(positive("-1").is_none());
        assert!(positive("1e9").is_none());
        assert!(positive("").is_none());
    }

    #[test]
    fn the_cache_round_trips_and_goes_stale() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join(ECB_CACHE);
        let (date, per_euro) = parse_ecb(ECB_SAMPLE).unwrap();
        let cache = Cache {
            fetched: 1_000,
            date,
            per_euro,
        };
        write_atomically(&path, &toml::to_string(&cache).unwrap()).unwrap();
        let read = read_cache(&path).unwrap();
        assert_eq!(read.per_euro, cache.per_euro);
        let at = |secs: u64| UNIX_EPOCH + Duration::from_secs(secs);
        assert!(!read.is_stale(at(1_000 + ECB_MAX_AGE.as_secs())));
        assert!(read.is_stale(at(1_001 + ECB_MAX_AGE.as_secs())));
    }

    #[test]
    fn minor_units_follow_iso_4217() {
        assert_eq!(minor_units("JPY"), 0);
        assert_eq!(minor_units("USD"), 2);
    }
}
