# Plans

## calc: units, currencies and crypto

calc converts and computes with units, then fiat currencies and crypto
currencies, whose rates are fetched in the background and cached so startup
never waits on the network. Decided 2026-11-13: rates from the European Central
Bank (fiat) and CoinGecko (crypto), fetched with the system's `curl` (no TLS
stack compiled into pikr).

| Phase                            | Status |
| -------------------------------- | ------ |
| 01 Units and conversions         | done   |
| 02 Fiat currencies (ECB)         | done   |
| 03 Crypto currencies (CoinGecko) | done   |
