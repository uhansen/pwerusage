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
- Build `syncweatherstations` with the shared WASI skill script and run it
  through its custom host; ordinary `wasmtime run` cannot satisfy its imports.
