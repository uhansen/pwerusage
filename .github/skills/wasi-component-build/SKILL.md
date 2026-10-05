---
name: wasi-component-build
description: Build and run Rust WebAssembly components, trying WASIp3 (wasm32-wasip3, WASI 0.3, component-model async) first and falling back to WASIp2 (wasm32-wasip2). Use whenever the user asks to compile, build, run, or test a Wasm/WASI component; mentions p2, p3, wasip3, WASI 0.3, or async components; or encounters WIT binding errors.
---

# WASI component build

Always use the scripts included with this skill; do not hand-write Cargo or
Wasmtime commands.

## Build

```bash
.github/skills/wasi-component-build/scripts/build.sh [--release] [extra cargo args]
```

The script tries `wasm32-wasip3` with nightly Rust, then nightly Rust with
`-Zbuild-std`, and finally falls back to `wasm32-wasip2`.

The last line of successful output is:

```text
RESULT target=<target> wasm=<path>
```

## Run

```bash
.github/skills/wasi-component-build/scripts/run.sh <path.wasm> [args...]
```

The script selects the Wasmtime flags based on the artifact path. WASIp3
requires Wasmtime 45 or newer.

## Report

- Tell the user which target was used.
- If the build fell back to WASIp2, show the WASIp3 error lines printed by the
  build script.

## Rules

- Never edit generated bindings. Change the `.wit` file and rebuild.
- Gate WASIp3-only Rust code with
  `#[cfg(all(target_os = "wasi", target_env = "p3"))]`.
- WASIp3 WIT uses `async func`, `stream<T>`, and `future<T>`.
- `wasi:io` no longer exists in WASIp3. Do not mix WASIp2 `wasi:io` imports
  into a WASIp3 world.
- Inspect component exports with `wasm-tools component wit <path.wasm>`.
