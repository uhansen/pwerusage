---
name: wasi-component-build
description: Build and run Rust WebAssembly components, trying WASIp3 (wasm32-wasip3, WASI 0.3, component-model async) first and falling back to WASIp2 (wasm32-wasip2). Use whenever the user asks to compile, build, run or test a Wasm/WASI component, mentions p2, p3, wasip3, WASI 0.3 or async components, or hits WIT binding errors.
compatibility: opencode
---

# WASI component build

Always use the scripts; don't hand-write cargo or wasmtime commands.

## Build
scripts/build.sh [--release] [extra cargo args]

Tries wasm32-wasip3 (nightly, then nightly + -Zbuild-std), falls back to
wasm32-wasip2. The last line of output is:
RESULT target=<target> wasm=<path>

## Run
scripts/run.sh <path.wasm> [args...]

Picks the right wasmtime flags from the path (p3 needs wasmtime >= 45).

## Report
- Tell the user which target was used.
- If it fell back to p2, show the p3 error lines the script printed.

## Rules
- Never edit generated bindings; change the .wit file and rebuild.
- p3-only code: #[cfg(all(target_os = "wasi", target_env = "p3"))]
- p3 WIT uses async func, stream<T>, future<T>; wasi:io no longer exists.
  Don't mix p2 wasi:io imports into a p3 world.
- Inspect exports: wasm-tools component wit <path.wasm>
