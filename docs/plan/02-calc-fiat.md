# 02 Fiat currencies (ECB)

- Currencies are units of the money dimension (`100 USD to EUR`, `€`, `$`, `£`,
  `¥` where unambiguous), with factors from the ECB's daily reference rates
  (`eurofxref-daily.xml`, about 30 currencies against EUR).
- Fetched on a background thread with the system `curl`, only in calc mode and
  only when the cache is older than a day; startup reads the cache file (under
  pikr's cache directory) and never waits. A fetch that lands while pikr is open
  re-ranks the rows.
- The row says how old the rates are when they are older than two days, and no
  currency result is shown without rates (no hardcoded fallback rates).

## Exit

Parsing tests against a saved ECB file, a test that a failed or missing `curl`
leaves the cache alone, and the windowed check.
