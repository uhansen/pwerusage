#!/usr/bin/env bash
set -euo pipefail

APP=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
TELEMETRY="$APP/../../components/saveeye-telemetry"
cd "$APP"

cargo build --locked --target wasm32-wasip2 --release -j 2
cargo build --locked --manifest-path "$TELEMETRY/Cargo.toml" --target wasm32-wasip2 --release -j 2
wac plug \
  --plug "$TELEMETRY/target/wasm32-wasip2/release/saveeye_telemetry.wasm" \
  "$APP/target/wasm32-wasip2/release/save_eye_edge_listener_app.wasm" \
  -o "$APP/composed.wasm"
wasm-tools component wit "$APP/composed.wasm" >/dev/null
