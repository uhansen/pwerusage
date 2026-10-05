# Copilot Instructions

This repository contains a Rust WebAssembly/WASI component in
`nearbyweatherstations/`.

## Commands

Run these commands from the repository root:

- Build the WASI component:
  `cd nearbyweatherstations && ../.github/skills/wasi-component-build/scripts/build.sh`
- Lint:
  `cd nearbyweatherstations && cargo clippy --all-targets -- -D warnings`
- Format check: `cd nearbyweatherstations && cargo fmt --check`
- Full test suite: `cd nearbyweatherstations && cargo test`
- Single test:
  `cd nearbyweatherstations && cargo test <test_name>`
- Inspect the built component:
  `cd nearbyweatherstations && wasm-tools component wit target/wasm32-wasip3/debug/nearbyweatherstations.wasm`

## Architecture and conventions

- `nearbyweatherstations/wit/world.wit` is the source of truth for the
  component interface. Never edit generated WIT bindings.
- `nearbyweatherstations/src/lib.rs` generates bindings with `wit-bindgen`,
  implements the exported `Guest` trait, and contains the host-testable
  distance and selection logic.
- The component targets WASIp3 first and falls back to WASIp2 through the
  repository's `wasi-component-build` skill script. Gate WASIp3-only Rust code
  with `#[cfg(all(target_os = "wasi", target_env = "p3"))]`.
- Coordinates use degrees and distances use kilometers. Validate latitude and
  longitude before calculating Haversine distance.
- Keep component export macros gated to `target_family = "wasm"` so native
  unit tests do not attempt to link component ABI symbols into a host library.
