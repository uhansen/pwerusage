#!/usr/bin/env bash
# Run a WASI component with flags matching its target.
# Usage: scripts/run.sh <path.wasm> [args...]
set -euo pipefail

WASM=${1:?usage: run.sh <path.wasm> [args...]}
shift
FLAGS=()

if [[ $WASM == *wasip3* ]]; then
  major=$(wasmtime -V | awk '{print $2}' | cut -d. -f1)
  if (( major < 45 )); then
    echo "wasmtime $major is too old for p3 (need >= 45)" >&2
    exit 1
  fi
  FLAGS=(-S p3=y -W component-model=y -W component-model-async=y)
fi

exec wasmtime run ${FLAGS[@]+"${FLAGS[@]}"} "$WASM" "$@"
