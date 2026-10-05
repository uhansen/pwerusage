#!/usr/bin/env bash
# Build a Rust WASI component: try wasm32-wasip3, fall back to wasm32-wasip2.
# Usage: scripts/build.sh [--release] [extra cargo args]
set -uo pipefail

PROFILE=debug
[[ "${1:-}" == "--release" ]] && PROFILE=release
LOG=$(mktemp)

result() {
  local wasm
  wasm=$(ls -t "target/$1/$PROFILE"/*.wasm 2>/dev/null | head -1)
  echo "RESULT target=$1 wasm=$wasm"
  exit 0
}

# 1. p3 with prebuilt target
if rustup +nightly target add wasm32-wasip3 >>"$LOG" 2>&1 &&
   cargo +nightly build --target wasm32-wasip3 "$@" >>"$LOG" 2>&1; then
  result wasm32-wasip3
fi

# 2. p3 with std built from source
if rustup +nightly component add rust-src >>"$LOG" 2>&1 &&
   cargo +nightly build --target wasm32-wasip3 \
     -Zbuild-std=std,panic_abort "$@" >>"$LOG" 2>&1; then
  result wasm32-wasip3
fi

echo "p3 failed, last errors:"
grep -E '^(error|warning: unused)' "$LOG" | tail -20 || tail -20 "$LOG"

# 3. p2 fallback
rustup target add wasm32-wasip2 >/dev/null 2>&1
cargo build --target wasm32-wasip2 "$@" || exit 1
result wasm32-wasip2
