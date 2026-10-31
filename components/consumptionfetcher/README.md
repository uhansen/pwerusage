# consumptionfetcher

Rust WASI component that matches a household address against the metering points
on an Eloverblik account, fetches daily electricity readings, and persists daily,
weekly, monthly and yearly consumption and available production. This is **not a
Spin app**. The component uses standard outbound `wasi:http`; its native Wasmtime
host implements the custom PostgreSQL `storage` import.

The [Customer API documentation](https://docs.eloverblik.dk/docs/api/customer)
links to the
[OpenAPI contract](https://api.eloverblik.dk/customerapi/swagger/customerapi-v1.0/swagger.json).
The component exchanges a portal **refresh token** (the API key) for an access
token using `GET /token`, then uses `api-version: 1.0` on authenticated calls.
An address alone does not grant access: link the relevant metering points in the
Eloverblik portal first.

## Build and run

From `components/consumptionfetcher/`:

```bash
../../.github/skills/wasi-component-build/scripts/build.sh
cargo build --manifest-path host/Cargo.toml
```

The shared script tries WASIp3 first, then falls back to WASIp2. Use its printed
artifact path; the host links both versions. WASIp3 HTTP is still experimental.
`wasip3` 0.9 and `wit-bindgen` 0.62 are deliberately aligned with WASI 0.3.0 and
the modern Rust WASIp3 task-local runtime. Do not downgrade just the bindings:
older context-slot-based async runtimes cannot run correctly on current WASIp3.

Create the database and a dedicated login **once** on your existing local server:

```bash
docker exec postgres18 psql -U postgres -d postgres -v ON_ERROR_STOP=1 \
  -c 'CREATE ROLE consumptionfetcher LOGIN'
docker exec postgres18 createdb -U postgres -O consumptionfetcher powerusage
```

This host follows the existing repository's local PostgreSQL connection pattern
(`NoTls`); it is intended for a trusted local server. If authentication requires
a password, supply it through an external PostgreSQL password file or connection
configuration, never a committed file. Remote PostgreSQL TLS is not implemented.

```bash
export DATABASE_URL='postgresql://consumptionfetcher@127.0.0.1:5432/powerusage'
read -rs -p 'Eloverblik portal refresh token: ' ELOVERBLIK_REFRESH_TOKEN
echo
export ELOVERBLIK_REFRESH_TOKEN

../../.github/skills/wasi-component-build/scripts/run.sh \
  --host host/target/debug/consumptionfetcher-host \
  target/wasm32-wasip3/debug/consumptionfetcher.wasm \
  --street 'Examplevej' --number '12' --postcode '2200' \
  --floor '1' --door 'th'
unset ELOVERBLIK_REFRESH_TOKEN
```

`--city`, `--floor` and `--door` are optional. Matching trims whitespace, collapses
repeated spaces and ignores case (including Danish letters), but does not guess
spelling, abbreviations or apartment numbers. If multiple apartments match,
specify floor and door. Moved-out points are excluded.

The host passes **only the refresh-token environment variable** to the guest,
not PostgreSQL credentials. HTTP is restricted to HTTPS on `api.eloverblik.dk`,
port 443 and the Customer API path; redirects are not followed. Tokens and
upstream response bodies are not printed in error messages.

## Fetch window and quantities

The WIT export is:

```wit
fetch: async func(address: address, date-from: option<string>,
    date-to: option<string>) -> result<fetch-summary, fetch-error>;
```

Dates use `YYYY-MM-DD`; `date-to` is **exclusive**. By default it is today in
Europe/Copenhagen, excluding today's unfinished readings. The first run starts
January 1 five years ago (the API retention boundary), restricted by the current
consumer's start date when supplied by the API. Later runs start seven days
before the last stored day. To re-fetch older corrections or use a smaller
initial window, pass `--date-from` and `--date-to`.

Requests use `Day` aggregation, split into at most 730 days per request, one
metering point per request. Requests are paced below the documented 120/minute
per-IP limit. 429 and 503 are retried at most three times with bounded backoff and
`Retry-After` support; token retries wait at least a minute (token limit: 2/minute
per IP). Multiple applications on the same IP still share the upstream limit.

Daily UTC interval boundaries are converted to Copenhagen calendar dates;
positions advance by **calendar day**, so DST days need not be 24 hours long.
Amounts use integer milli-kWh, avoiding floating-point rounding; SQL exposes
exact decimal `kwh`. Weeks start Monday (ISO weeks); months and years are calendar
periods. Period ends are exclusive.

The component merges new/corrected daily readings into **all existing daily
history before rebuilding rollups**. Thus an incremental update never overwrites
a month or year with just the overlap window. Persistence is atomic and
idempotent. The host holds a database-wide advisory lock across history reads and
writes so concurrent host runs cannot persist stale aggregates.

Missing readings (`A02`) are stored with a null amount, not zero. Aggregate
amounts become null if any included daily amount is null. Partial periods and
gaps are marked `complete=false`, `quality=A05`; otherwise available quantities
are summed. Estimated readings retain `A03`. No-data API results (`30015`) are
counted in `unavailable_periods` and do not erase history. Other per-meter errors
fail the run without persisting partial results. Readings may lag several days;
the component does not invent unavailable data.

## Production and solar panels

Parent and child points are discovered automatically. Each point retains its
original type code and its own independent records:

| Direction | Supported type codes |
|---|---|
| consumption | E17, D07 (grid import), D12 (total consumption) |
| production | E18, D01, D05 (net production), D09, D18 |
| grid-export | D04 (surplus), D06 (supply to grid), D11 (net to grid) |

This fetches the available production-related readings for a solar-equipped
household, but these type codes alone do **not** prove that the source is solar
rather than another technology. Grid export is not gross solar generation:
self-consumed generation cannot be reconstructed without suitable production
metering. Do not sum parent and child consumption readings or different
production/export types together; they can overlap. Views deliberately retain
metering-point identity rather than combine unlike measurements.

## Database and validation

The host applies `migrations/001_usage.sql` automatically. `metering_points`
stores the matched address and type; `usage_periods` stores all four granularities.
Views: `daily_usage`, `weekly_usage`, `monthly_usage`, `yearly_usage`.

```sql
SELECT metering_point_id, direction, period_start, period_end, kwh, complete, quality
FROM monthly_usage
ORDER BY metering_point_id, period_start;
```

```bash
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test --manifest-path host/Cargo.toml
cargo clippy --manifest-path host/Cargo.toml --all-targets -- -D warnings
cargo fmt --manifest-path host/Cargo.toml --check
wasm-tools component wit target/wasm32-wasip3/debug/consumptionfetcher.wasm
```

An opt-in integration test invokes the **compiled component**, mocks the real
`wasi:http` boundary, and writes to an empty disposable PostgreSQL database.
It verifies consumption/production/export, all granularities, corrections,
idempotence, upstream failure and transaction rollback. No real token is needed.
Run from this component directory (Cargo starts the test in `host/`):

```bash
TEST_DATABASE_URL='postgresql://postgres@127.0.0.1:5432/your_empty_test_database' \
TEST_COMPONENT_PATH='../target/wasm32-wasip3/debug/consumptionfetcher.wasm' \
cargo test --manifest-path host/Cargo.toml compiled_component_persists -- --ignored
```

The same test can use the WASIp2 artifact. Real-account verification requires
your portal refresh token and a linked household address.
