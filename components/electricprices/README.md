# electricprices

Rust WASI component for Electricity Maps day-ahead electricity prices. Input is
an electricity supplier (name, per-kWh markup, markup currency, VAT flag) and
either coordinates in degrees or an Electricity Maps zone key such as `DK-DK1`.
The native Wasmtime host persists prices in the existing `powerusage` PostgreSQL
database. **Not a Spin app.**

## API and scope

The [Electricity Maps developer hub](https://app.electricitymaps.com/api) requires
an API subscription/token. Requests go to `https://api.electricitymaps.com` with
an `auth-token` header, using:

| Endpoint | Purpose |
|---|---|
| `/v3/zone?lat=...&lon=...` | Resolve coordinates to a zone (`zoneKey`) |
| `/v3/price-day-ahead/latest` | Latest available price |
| `/v3/price-day-ahead/past-range` | Historical prices in five-day windows |
| `/v4/price-day-ahead/actual` | Published prices for today and tomorrow |

Price requests use `zone`, `temporalGranularity=hourly`, and `start`/`end` UTC
timestamps for ranges. The expected price fields are `datetime`, `value`, `unit`,
and `zone`; ranges have a `data` array. Responses must match the requested zone,
range and hourly resolution. The undocumented provisional v3 price forecast
route is **not** used.

`forecast` in storage means a **published future day-ahead interval**, not a
statistical prediction. Tomorrow may not yet be published; an empty successful
range is allowed. Null values are counted as unavailable and not inserted, and
never erase previously stored prices or become zero. HTTP/authorization/schema
errors fail the run explicitly, with no partial persistence.

The supplier markup is supplied by the caller, **not looked up at Electricity
Maps**. Network tariffs, electricity taxes, fixed monthly fees and supplier
contracts are not fetched. The total is only spot plus markup, optionally with
25% VAT; it is not a complete household tariff.

The developer documentation was inaccessible during implementation. Endpoint
and schema choices are based on public integrations; tests use synthetic
responses. **Live authenticated price access and your plan's history depth
remain unverified.** 401/403 can indicate an invalid token or insufficient plan
access. Unexpected statuses, unavailable endpoints and unsupported currencies
are reported rather than silently switching sources.

## Build and run

From `components/electricprices/`:

```bash
../../.github/skills/wasi-component-build/scripts/build.sh
cargo build --manifest-path host/Cargo.toml
```

The script builds WASIp3 first and falls back to WASIp2. Use the artifact path
printed in its `RESULT` line. The host links both WASI and HTTP versions.
`wit/world.wit` is the interface source of truth; never edit generated bindings.
`wasip3` 0.9 and `wit-bindgen` 0.62 are aligned with WASI 0.3.0 and Wasmtime 48.

For an existing `powerusage` database, create a separate login **once**:

```bash
docker exec postgres18 psql -U postgres -d powerusage -v ON_ERROR_STOP=1 \
  -c 'CREATE ROLE electricprices LOGIN' \
  -c 'GRANT CONNECT ON DATABASE powerusage TO electricprices' \
  -c 'GRANT USAGE, CREATE ON SCHEMA public TO electricprices'
```

The host creates its own table/view, with an idempotent migration that does not
touch consumption-fetcher tables. It holds a separate session advisory lock
across migration, history reads, HTTP fetches and transactional upserts.
PostgreSQL uses `NoTls`, matching this repository's trusted-local-server pattern.
For password authentication use an external PostgreSQL password file; do not
commit credentials. Remote PostgreSQL TLS is not implemented.

```bash
export DATABASE_URL='postgresql://electricprices@127.0.0.1:5432/powerusage'
read -rs -p 'Electricity Maps API token: ' ELECTRICITYMAPS_API_TOKEN
echo
export ELECTRICITYMAPS_API_TOKEN

../../.github/skills/wasi-component-build/scripts/run.sh \
  --host host/target/debug/electricprices-host \
  target/wasm32-wasip3/debug/electricprices.wasm \
  --provider-name 'Example supplier' --markup 0.10 --markup-currency DKK \
  --zone DK-DK2 --vat
unset ELECTRICITYMAPS_API_TOKEN
```

Use `--lat 55.7 --lon 12.5` instead of `--zone` for coordinates. Both coordinates
are required; they cannot be combined with a zone. The host passes only
`ELECTRICITYMAPS_API_TOKEN` and `EUR_DKK_RATE` into the guest, never database
credentials. HTTP is restricted to GET, HTTPS, port 443 and the four endpoint
paths above. Tokens and upstream bodies are not printed; redirects are rejected.
429 and 5xx receive up to three retries with bounded `Retry-After`/backoff.

## Windows, units and persistence

The WIT export is:

```wit
fetch: async func(provider: provider, position: position,
    history-from: option<string>, history-to: option<string>)
    -> result<fetch-summary, fetch-error>;
```

History dates are `YYYY-MM-DD` in Europe/Copenhagen; `history-to` is exclusive,
defaults to today, and cannot be in the future. An explicit `--history-from`
overrides incremental fetching. Without it the first run requests the preceding
30 calendar days; subsequent runs overlap the last stored historical hour by
24 hours. The watermark is isolated by zone **and provider** and does not include
future prices. Today/tomorrow and latest are fetched on every run, independently
of the historical window.

Prices are explicitly **hourly**, not quarter-hour settlement prices. UTC
timestamps distinguish repeated DST hours; Copenhagen days can contain 23 or 25
hours. `daily_prices` groups by Copenhagen date and VAT flag, reports averages,
minimum, maximum, available-hour count and upcoming-hour count. A partial day
is not presented as a complete day, and the average is not consumption-weighted.

Supported raw units are EUR/MWh, DKK/MWh, EUR/kWh and DKK/kWh:

```text
spot_DKK/kWh = price_EUR/MWh / 1000 * EUR_DKK_RATE
markup_DKK/kWh = markup * (EUR_DKK_RATE if markup currency is EUR, else 1)
total_DKK/kWh = (spot_DKK/kWh + markup_DKK/kWh) * (1.25 if --vat, else 1)
```

`EUR_DKK_RATE` defaults to **7.46**, overridable with the environment variable or
`--eur-dkk-rate`. No live currency-rate service is consulted. Markups are
pre-VAT; default is **no VAT**. Negative spot prices and finite negative markups
are allowed. Raw prices, units, exchange rate, supplier inputs and derived
DKK/kWh amounts are preserved as double precision values, not invoice-rounded.

`electricity_prices` is keyed by `(zone, datetime, provider_name)`. History
replaces latest/upcoming rows, but upcoming snapshots cannot overwrite
historical corrections. Changing a supplier's markup/VAT/rate updates the
fetched window; pass an explicit historical start to recalculate older rows.
An unavailable historical hour older than the overlap likewise requires an
explicit backfill.

## Validation

```bash
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test --manifest-path host/Cargo.toml
cargo clippy --manifest-path host/Cargo.toml --all-targets -- -D warnings
cargo fmt --manifest-path host/Cargo.toml --check
wasm-tools validate target/wasm32-wasip3/debug/electricprices.wasm
wasm-tools component wit target/wasm32-wasip3/debug/electricprices.wasm
```

An ignored integration test exercises the compiled component, mocked `wasi:http`
and real PostgreSQL (coordinate and zone inputs, conversion, missing values,
corrections, idempotency, failure atomicity and history precedence). Supply
`TEST_COMPONENT_PATH` and `TEST_DATABASE_URL` pointing to a **disposable empty
database**, then run:

```bash
cargo test --manifest-path host/Cargo.toml \
  compiled_component_persists_corrects_and_is_atomic -- --ignored --nocapture
```

The test leaves its tables in that disposable database; remove it after testing.
