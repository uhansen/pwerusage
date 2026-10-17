#!/usr/bin/env bash
set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
SYNC_ROOT="$ROOT/../../components/syncweatherstations"

cd "$ROOT"

cargo build --target wasm32-wasip2 --release
cargo build --manifest-path adapter/Cargo.toml --target wasm32-wasip2 --release
(
  cd "$SYNC_ROOT"
  cargo build --target wasm32-wasip2 --release
)

TMP_SYNC=$(mktemp "$ROOT/.syncweatherstations-plugged.XXXXXX.wasm")
trap 'rm -f "$TMP_SYNC"' EXIT

wac plug \
  --plug "$ROOT/target/wasm32-wasip2/release/syncweatherstations_adapter.wasm" \
  "$SYNC_ROOT/target/wasm32-wasip2/release/syncweatherstations.wasm" \
  -o "$TMP_SYNC"

wac plug \
  --plug "$TMP_SYNC" \
  "$ROOT/target/wasm32-wasip2/release/syncweatherstations_cron.wasm" \
  -o "$ROOT/composed.wasm"

wasm-tools component wit "$ROOT/composed.wasm" >/dev/null
