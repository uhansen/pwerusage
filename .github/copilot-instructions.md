# Copilot Instructions

This repository contains Rust WebAssembly/WASI components under `components/`.

## Commands

Run these commands from the repository root:

- Build the WASI component:
  `cd components/nearbyweatherstations && ../../.github/skills/wasi-component-build/scripts/build.sh`
- Lint:
  `cd components/nearbyweatherstations && cargo clippy --all-targets -- -D warnings`
- Format check: `cd components/nearbyweatherstations && cargo fmt --check`
- Full test suite: `cd components/nearbyweatherstations && cargo test`
- Single test:
  `cd components/nearbyweatherstations && cargo test <test_name>`
- Inspect the built component:
  `cd components/nearbyweatherstations && wasm-tools component wit target/wasm32-wasip3/debug/nearbyweatherstations.wasm`
- Build the station sync component:
  `cd components/syncweatherstations && ../../.github/skills/wasi-component-build/scripts/build.sh`
- Test/lint the station sync component:
  `cd components/syncweatherstations && cargo test && cargo clippy --all-targets -- -D warnings`
- Build/test the station sync host:
  `cd components/syncweatherstations && cargo test --manifest-path host/Cargo.toml`
- Run the station sync host:
  `cd components/syncweatherstations && ../../.github/skills/wasi-component-build/scripts/run.sh --host host/target/debug/syncweatherstations-host <component-path>`
- Build the consumption fetcher:
  `cd components/consumptionfetcher && ../../.github/skills/wasi-component-build/scripts/build.sh`
- Test/lint/format the consumption fetcher:
  `cd components/consumptionfetcher && cargo test && cargo clippy --all-targets -- -D warnings && cargo fmt --check`
- Build the consumption fetcher host:
  `cd components/consumptionfetcher && cargo build --manifest-path host/Cargo.toml`
- Test/lint/format the consumption fetcher host:
  `cd components/consumptionfetcher && cargo test --manifest-path host/Cargo.toml && cargo clippy --manifest-path host/Cargo.toml --all-targets -- -D warnings && cargo fmt --manifest-path host/Cargo.toml --check`
- Run the consumption fetcher host (with `ELOVERBLIK_REFRESH_TOKEN` and `DATABASE_URL` set):
  `cd components/consumptionfetcher && ../../.github/skills/wasi-component-build/scripts/run.sh --host host/target/debug/consumptionfetcher-host <component-path> --street Examplevej --number 12 --postcode 2200`
- Build the Spin cron sync app:
  `cd apps/syncweatherstations-cron && ./build.sh`
- Build the Spin cron sync app with Spin:
  `cd apps/syncweatherstations-cron && spin build`
- Run the Spin cron sync app:
  `cd apps/syncweatherstations-cron && SPIN_VARIABLE_DB_URL='postgresql://weatherstations_sync@127.0.0.1:5432/weatherstations' spin up`

## Architecture and conventions

- `components/nearbyweatherstations/wit/world.wit` is the source of truth for the
  component interface. Never edit generated WIT bindings.
- `components/nearbyweatherstations/src/lib.rs` generates bindings with `wit-bindgen`,
  implements the exported `Guest` trait, and contains the host-testable
  distance and selection logic.
- The component targets WASIp3 first and falls back to WASIp2 through the
  repository's `wasi-component-build` skill script. Gate WASIp3-only Rust code
  with `#[cfg(all(target_os = "wasi", target_env = "p3"))]`.
- Coordinates use degrees and distances use kilometers. Validate latitude and
  longitude before calculating Haversine distance.
- Keep component export macros gated to `target_family = "wasm"` so native
  unit tests do not attempt to link component ABI symbols into a host library.
- `components/syncweatherstations/` owns DMI pagination, parsing, validation,
  and retry decisions. Its native Rust host owns restricted HTTPS transport,
  Wasmtime integration, PostgreSQL credentials, migrations, and transactions.
- `apps/syncweatherstations-cron/` composes the `syncweatherstations` component
  with a Spin adapter and cron handler; the final Spin app reuses the same DMI
  sync logic and persists to PostgreSQL through Spin's outbound Postgres API.
- Build `syncweatherstations` with the shared WASI skill script and run it
  through its custom host; ordinary `wasmtime run` cannot satisfy its imports.
- `components/consumptionfetcher/` owns Eloverblik token exchange, structured
  household-address matching, daily parsing, and day/week/month/year rollups.
  It uses standard `wasi:http`; its native host owns PostgreSQL, migrations,
  and the custom `storage` import. It is not a Spin app.
- Consumption fetcher quantities are integer milli-kWh. Merge persisted daily
  history before recomputing rollups; never treat missing quantities as zero.
  Preserve point type and identity: grid export is not gross solar production.
