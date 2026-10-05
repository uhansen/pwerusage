#!/usr/bin/env bash
# Run a WASI component with flags matching its target.
# Usage: scripts/run.sh [--host <host-binary>] <path.wasm> [args...]
set -euo pipefail

if [[ ${1:-} == "--host" ]]; then
  HOST=${2:?usage: run.sh --host <host-binary> <path.wasm> [args...]}
  WASM=${3:?usage: run.sh --host <host-binary> <path.wasm> [args...]}
  shift 3
  exec "$HOST" --component "$WASM" "$@"
fi

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
