# 03 Crypto currencies (CoinGecko)

- `BTC`, `ETH`, `SOL` and a fixed list of the largest coins become money units,
  priced from CoinGecko's public `simple/price` endpoint (no key) in EUR so they
  share phase 02's base.
- Same background fetch and cache as phase 02, refreshed when older than an
  hour; the rate limit means one request per refresh for the whole list.

## Exit

Parsing tests against a saved response, and the windowed check.
