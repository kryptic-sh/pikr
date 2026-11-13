//! Exchange rates: read from a cache on disk at once, refreshed from the
//! network on a background thread so calc never waits for it.
//!
//! Fiat rates are the European Central Bank's daily reference rates
//! ([`ECB_URL`]), quoted against the euro; crypto prices are CoinGecko's, in
//! euros ([`COINGECKO_URL`]). Both are fetched with the system's `curl`, so
//! pikr carries no TLS stack of its own; without `curl`, or offline, the
//! cached rates stay in use and a first run has none.

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

/// CoinGecko's public price endpoint (no key); the coin ids are appended.
const COINGECKO_URL: &str = "https://api.coingecko.com/api/v3/simple/price?vs_currencies=eur&ids=";

/// How old cached rates may get before a refresh. The ECB publishes once a
/// working day, around 16:00 CET; crypto prices move all day, and the
/// public CoinGecko endpoint allows a few requests a minute.
const ECB_MAX_AGE: Duration = Duration::from_secs(12 * 60 * 60);
const COINGECKO_MAX_AGE: Duration = Duration::from_secs(15 * 60);

/// How long a fetch may take before `curl` gives up.
const FETCH_TIMEOUT_SECS: &str = "15";

/// The most bytes a rates response is read to; both are a few KB.
const MAX_RESPONSE: u64 = 1024 * 1024;

/// Currencies whose amounts have no minor unit (ISO 4217 exponent 0), shown
/// without decimals.
const WHOLE_CURRENCIES: &[&str] = &["JPY", "KRW", "ISK"];

/// Decimals a fiat amount is shown with, and a crypto one (a bitcoin's
/// smallest unit, the satoshi, is 10^-8).
const FIAT_DECIMALS: u32 = 2;
const CRYPTO_DECIMALS: u32 = 8;

/// Currency symbols and the code each stands for, where the symbol is not
/// shared by several currencies in everyday use.
const SYMBOLS: &[(&str, &str)] = &[
    ("€", "EUR"),
    ("$", "USD"),
    ("£", "GBP"),
    ("¥", "JPY"),
    ("₹", "INR"),
];

/// The coins calc prices: CoinGecko's id for each, and its ticker.
const COINS: &[(&str, &str)] = &[
    ("bitcoin", "BTC"),
    ("ethereum", "ETH"),
    ("tether", "USDT"),
    ("binancecoin", "BNB"),
    ("solana", "SOL"),
    ("ripple", "XRP"),
    ("usd-coin", "USDC"),
    ("cardano", "ADA"),
    ("dogecoin", "DOGE"),
    ("tron", "TRX"),
    ("the-open-network", "TON"),
    ("avalanche-2", "AVAX"),
    ("polkadot", "DOT"),
    ("chainlink", "LINK"),
    ("litecoin", "LTC"),
    ("bitcoin-cash", "BCH"),
    ("stellar", "XLM"),
    ("monero", "XMR"),
    ("cosmos", "ATOM"),
    ("shiba-inu", "SHIB"),
];

/// Where a rate comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Ecb,
    CoinGecko,
}

impl Source {
    fn cache_name(self) -> &'static str {
        match self {
            Self::Ecb => "rates-ecb.toml",
            Self::CoinGecko => "rates-coingecko.toml",
        }
    }

    fn max_age(self) -> Duration {
        match self {
            Self::Ecb => ECB_MAX_AGE,
            Self::CoinGecko => COINGECKO_MAX_AGE,
        }
    }

    fn url(self) -> String {
        match self {
            Self::Ecb => ECB_URL.to_owned(),
            Self::CoinGecko => {
                let ids: Vec<&str> = COINS.iter().map(|(id, _)| *id).collect();
                format!("{COINGECKO_URL}{}", ids.join(","))
            }
        }
    }

    /// The rates in a response from this source, as euros per unit.
    fn parse(self, body: &str) -> Option<(String, HashMap<String, String>)> {
        match self {
            Self::Ecb => parse_ecb(body),
            Self::CoinGecko => Some((String::new(), parse_coingecko(body)?)),
        }
    }
}

/// One source's rates: euros per unit of each currency.
#[derive(Clone, Debug, Default)]
struct Table {
    euros: HashMap<String, BigRational>,
    /// The day the source dates them (the ECB's), else empty.
    date: String,
    /// Seconds since the Unix epoch when they were fetched.
    fetched: u64,
}

/// The rates calc converts money with.
#[derive(Clone, Debug, Default)]
pub struct Rates {
    fiat: Table,
    crypto: Table,
}

impl Rates {
    /// The currency named `name` (`USD`, `usd`, `$`, `BTC`): its code, the
    /// euros one unit of it is worth, and where that came from, if there is
    /// a rate for it. The euro needs none.
    pub fn currency(&self, name: &str) -> Option<(String, BigRational, Source)> {
        let code = SYMBOLS
            .iter()
            .find(|(symbol, _)| *symbol == name)
            .map_or_else(|| name.to_ascii_uppercase(), |(_, code)| (*code).to_owned());
        if code == "EUR" {
            return Some((code, BigRational::from_integer(1.into()), Source::Ecb));
        }
        if let Some(euros) = self.fiat.euros.get(&code) {
            return Some((code, euros.clone(), Source::Ecb));
        }
        let euros = self.crypto.euros.get(&code)?;
        Some((code, euros.clone(), Source::CoinGecko))
    }

    /// What the row says about the rates from `sources`, at `now`.
    pub fn note(&self, sources: &[Source], now: SystemTime) -> Option<String> {
        let mut parts = Vec::new();
        if sources.contains(&Source::Ecb) && !self.fiat.date.is_empty() {
            parts.push(format!("ECB {}", self.fiat.date));
        }
        if sources.contains(&Source::CoinGecko) && self.crypto.fetched > 0 {
            let fetched = UNIX_EPOCH + Duration::from_secs(self.crypto.fetched);
            let age = now.duration_since(fetched).unwrap_or_default();
            parts.push(format!("CoinGecko {} ago", rough_age(age)));
        }
        (!parts.is_empty()).then(|| format!("rates: {}", parts.join(", ")))
    }
}

/// `age` in the largest whole unit that fits (`3 min`, `2 h`, `5 days`).
fn rough_age(age: Duration) -> String {
    let minutes = age.as_secs() / 60;
    match minutes {
        0 => "moments".to_owned(),
        1..60 => format!("{minutes} min"),
        60..2880 => format!("{} h", minutes / 60),
        _ => format!("{} days", minutes / 1440),
    }
}

/// Decimals an amount in `code` is shown with.
pub fn decimals(code: &str) -> u32 {
    if COINS.iter().any(|(_, ticker)| *ticker == code) {
        CRYPTO_DECIMALS
    } else if WHOLE_CURRENCIES.contains(&code) {
        0
    } else {
        FIAT_DECIMALS
    }
}

/// What a cache file holds.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct Cache {
    /// Seconds since the Unix epoch when it was fetched.
    fetched: u64,
    date: String,
    /// The ECB's: units of each currency per euro, as it wrote them.
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    per_euro: HashMap<String, String>,
    /// CoinGecko's: euros per unit of each coin, as it wrote them.
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    euros: HashMap<String, String>,
}

impl Cache {
    fn table(&self) -> Table {
        let mut euros: HashMap<String, BigRational> = self
            .per_euro
            .iter()
            .filter_map(|(code, rate)| Some((code.clone(), positive(rate)?.recip())))
            .collect();
        euros.extend(
            self.euros
                .iter()
                .filter_map(|(code, price)| Some((code.clone(), positive(price)?))),
        );
        Table {
            euros,
            date: self.date.clone(),
            fetched: self.fetched,
        }
    }

    fn is_stale(&self, max_age: Duration, now: SystemTime) -> bool {
        let fetched = UNIX_EPOCH + Duration::from_secs(self.fetched);
        now.duration_since(fetched)
            .map_or(true, |age| age > max_age)
    }
}

/// A rate as written (`1.0812`, `3.85e-06`), if it is a positive decimal.
fn positive(text: &str) -> Option<BigRational> {
    let text = text.trim();
    let (mantissa, exponent) = text
        .split_once(['e', 'E'])
        .map_or((text, Some(0)), |(m, e)| (m, e.parse::<i64>().ok()));
    let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let digits = |s: &str| s.bytes().all(|b| b.is_ascii_digit());
    if whole.is_empty() || !digits(whole) || !digits(fraction) {
        return None;
    }
    let value = decimal(whole, fraction, exponent?)?;
    (value.is_positive() && !value.is_zero()).then_some(value)
}

fn cache_path(source: Source) -> Option<PathBuf> {
    crate::picker::state_file_path(source.cache_name())
}

fn read_cache(path: &Path) -> Option<Cache> {
    toml::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

/// The cached rates, read now; empty where there are none yet.
pub fn load() -> Rates {
    let table = |source| {
        cache_path(source)
            .and_then(|path| read_cache(&path))
            .map(|cache| cache.table())
            .unwrap_or_default()
    };
    Rates {
        fiat: table(Source::Ecb),
        crypto: table(Source::CoinGecko),
    }
}

/// Starts refreshing whichever cached rates are missing or stale, on a
/// thread of its own. The receiver gets `()` each time new rates are on
/// disk, and sees the sender go when the refresh is over; `None` when no
/// refresh was needed.
pub fn refresh_in_background() -> Option<mpsc::Receiver<()>> {
    let now = SystemTime::now();
    let stale: Vec<(Source, PathBuf)> = [Source::Ecb, Source::CoinGecko]
        .into_iter()
        .filter_map(|source| Some((source, cache_path(source)?)))
        .filter(|(source, path)| {
            read_cache(path).is_none_or(|cache| cache.is_stale(source.max_age(), now))
        })
        .collect();
    if stale.is_empty() {
        return None;
    }
    let (done, landed) = mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name("pikr-rates".into())
        .spawn(move || {
            for (source, path) in stale {
                match refresh(source, &path) {
                    Ok(()) => {
                        let _ = done.send(());
                    }
                    Err(e) => tracing::debug!(error = %e, ?source, "rates not refreshed"),
                }
            }
        });
    if let Err(e) = spawned {
        tracing::debug!(error = %e, "cannot start the exchange-rate refresh");
        return None;
    }
    Some(landed)
}

/// Fetches `source`'s rates and replaces the cache at `path` with them.
fn refresh(source: Source, path: &Path) -> anyhow::Result<()> {
    let body = fetch(&source.url())?;
    let (date, rates) = source
        .parse(&body)
        .ok_or_else(|| anyhow::anyhow!("the response did not parse"))?;
    let fetched = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let mut cache = Cache {
        fetched,
        date,
        ..Cache::default()
    };
    match source {
        Source::Ecb => cache.per_euro = rates,
        Source::CoinGecko => cache.euros = rates,
    }
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

/// The euro prices in CoinGecko's `simple/price` answer, by ticker:
/// `{"bitcoin":{"eur":62110},"shiba-inu":{"eur":3.85e-06}}`. Only the coins
/// in [`COINS`] are read; `None` when none of them is there.
fn parse_coingecko(json: &str) -> Option<HashMap<String, String>> {
    let mut prices = HashMap::new();
    for (id, ticker) in COINS {
        let Some(at) = json.find(&format!("\"{id}\"")) else {
            continue;
        };
        let rest = &json[at + id.len() + 2..];
        let Some(eur) = rest.find("\"eur\"") else {
            continue;
        };
        // The price belongs to this coin only if no other object starts
        // between the id and the key.
        if rest[..eur].contains('}') {
            continue;
        }
        let number: String = rest[eur + 5..]
            .trim_start_matches([' ', ':'])
            .chars()
            .take_while(|c| c.is_ascii_digit() || matches!(c, '.' | 'e' | 'E' | '-' | '+'))
            .collect();
        if positive(&number).is_some() {
            prices.insert((*ticker).to_owned(), number);
        }
    }
    (!prices.is_empty()).then_some(prices)
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

    /// A CoinGecko answer as it came on 2026-11-13.
    const COINGECKO_SAMPLE: &str = r#"{"bitcoin":{"eur":62110},"ethereum":{"eur":1746.04},"shiba-inu":{"eur":3.85e-06},"tether":{"eur":0.903347}}"#;

    /// Rates from the samples, fetched at second 1000, for other modules'
    /// tests.
    pub fn sample_rates() -> Rates {
        let (date, per_euro) = parse_ecb(ECB_SAMPLE).unwrap();
        let fiat = Cache {
            fetched: 1_000,
            date,
            per_euro,
            ..Cache::default()
        };
        let crypto = Cache {
            fetched: 1_000,
            euros: parse_coingecko(COINGECKO_SAMPLE).unwrap(),
            ..Cache::default()
        };
        Rates {
            fiat: fiat.table(),
            crypto: crypto.table(),
        }
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
    fn the_coingecko_answer_parses() {
        let prices = parse_coingecko(COINGECKO_SAMPLE).unwrap();
        assert_eq!(prices["BTC"], "62110");
        assert_eq!(prices["SHIB"], "3.85e-06");
        assert_eq!(prices.len(), 4);
        // A coin with no price of its own does not take the next one's.
        let partial = parse_coingecko(r#"{"bitcoin":{},"ethereum":{"eur":1746.04}}"#).unwrap();
        assert_eq!(partial.get("BTC"), None);
        assert_eq!(partial["ETH"], "1746.04");
        assert_eq!(parse_coingecko(r#"{"status":{"error_code":429}}"#), None);
    }

    #[test]
    fn currencies_by_code_symbol_and_case() {
        let rates = sample_rates();
        let usd = BigRational::new(10000.into(), 10812.into());
        assert_eq!(
            rates.currency("USD"),
            Some(("USD".into(), usd.clone(), Source::Ecb))
        );
        assert_eq!(
            rates.currency("usd"),
            Some(("USD".into(), usd, Source::Ecb))
        );
        let code = |name| rates.currency(name).map(|(code, _, _)| code);
        assert_eq!(code("$").as_deref(), Some("USD"));
        assert_eq!(code("€").as_deref(), Some("EUR"));
        assert_eq!(code("btc").as_deref(), Some("BTC"));
        assert_eq!(
            rates.currency("BTC").map(|(_, _, s)| s),
            Some(Source::CoinGecko)
        );
        assert_eq!(rates.currency("CHF"), None);
        assert_eq!(Rates::default().currency("USD"), None);
        assert!(Rates::default().currency("EUR").is_some());
    }

    #[test]
    fn the_note_names_each_source_used() {
        let rates = sample_rates();
        let at = UNIX_EPOCH + Duration::from_secs(1_000 + 3 * 60 * 60);
        assert_eq!(
            rates.note(&[Source::Ecb], at).as_deref(),
            Some("rates: ECB 2026-11-12")
        );
        assert_eq!(
            rates.note(&[Source::Ecb, Source::CoinGecko], at).as_deref(),
            Some("rates: ECB 2026-11-12, CoinGecko 3 h ago")
        );
        assert_eq!(rates.note(&[], at), None);
        assert_eq!(rough_age(Duration::from_secs(30)), "moments");
        assert_eq!(rough_age(Duration::from_secs(5 * 60)), "5 min");
        assert_eq!(rough_age(Duration::from_secs(3 * 86400)), "3 days");
    }

    #[test]
    fn bad_rates_are_dropped() {
        assert!(positive("1.0812").is_some());
        assert_eq!(
            positive("3.85e-06"),
            Some(BigRational::new(385.into(), 100_000_000.into()))
        );
        assert!(positive("0").is_none());
        assert!(positive("-1").is_none());
        assert!(positive("1e").is_none());
        assert!(positive("").is_none());
        assert!(positive("NaN").is_none());
    }

    #[test]
    fn the_cache_round_trips_and_goes_stale() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join(Source::Ecb.cache_name());
        let (date, per_euro) = parse_ecb(ECB_SAMPLE).unwrap();
        let cache = Cache {
            fetched: 1_000,
            date,
            per_euro,
            ..Cache::default()
        };
        write_atomically(&path, &toml::to_string(&cache).unwrap()).unwrap();
        let read = read_cache(&path).unwrap();
        assert_eq!(read.per_euro, cache.per_euro);
        let at = |secs: u64| UNIX_EPOCH + Duration::from_secs(secs);
        assert!(!read.is_stale(ECB_MAX_AGE, at(1_000 + ECB_MAX_AGE.as_secs())));
        assert!(read.is_stale(ECB_MAX_AGE, at(1_001 + ECB_MAX_AGE.as_secs())));
    }

    #[test]
    fn decimals_follow_the_currency() {
        assert_eq!(decimals("JPY"), 0);
        assert_eq!(decimals("USD"), 2);
        assert_eq!(decimals("BTC"), 8);
    }
}
