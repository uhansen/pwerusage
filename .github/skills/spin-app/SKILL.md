---
name: spin-app
description: Create, extend, build and run Spin (spinframework.dev) WebAssembly applications — scaffolding with spin new/spin add, editing spin.toml, writing Rust or Go HTTP/Redis components, granting capabilities, and deploying to registries or SpinKube. Use whenever the user mentions Spin, spin.toml, Fermyon, SpinKube, or serverless Wasm/WASI components.
---

# Building Spin apps (Spin 4.x)

Spin (https://spinframework.dev) runs event-driven apps built from WebAssembly components.
An app = `spin.toml` manifest + one or more Wasm components. Docs: https://spinframework.dev/v4/

## Ground rules

- **Start from a template.** Use `spin new` / `spin add`; do not hand-write a manifest or SDK boilerplate unless asked.
- **Always pass `--accept-defaults` (or `-a`) and the values as flags.** `spin new` is interactive and will hang otherwise.
- **Keep code minimal.** One handler, few dependencies, no frameworks unless the user wants routing.
- **Deny by default.** Every capability (network, KV, SQLite, variables, files) must be granted in `spin.toml`. Most runtime errors come from a missing grant.
- **Verify by running it**: `spin build`, then `spin up`, then `curl`.

## Prerequisites (check first)

```bash
spin --version                      # expect 4.x
spin templates list                 # templates installed?
spin templates install --git https://github.com/spinframework/spin --update   # if missing
```

Per language:
- **Rust:** `rustup target add wasm32-wasip2`
- **Go:** Go ≥ 1.25.5 (TinyGo is no longer needed); `componentize-go` as a `go.mod` tool (template sets this up)
- **JS/TS / Python:** see the language guides at https://spinframework.dev/v4/language-support-overview

## Pick the right template

| Need | Template |
|---|---|
| HTTP, Rust / Go / TS / JS / Python | `http-rust`, `http-go`, `http-ts`, `http-js`, `http-py` |
| Redis subscriber | `redis-rust`, `redis-go` |
| App shell, add components later | `http-empty` |
| Host that only runs Spin 3 / WASI P2 (e.g. older SpinKube/containerd-shim) | the `-p2` variant, e.g. `http-rust-p2` |

The default Rust, Go and Python templates target Spin 4 / WASI Preview 3. **Ask or check the deployment target** before choosing; if unsure and the target is Kubernetes, use the `-p2` variant.

## Create an app

```bash
spin new -t http-rust my-app --accept-defaults --value http-path=/...
cd my-app
spin build --up        # builds and serves on http://127.0.0.1:3000
curl -i localhost:3000/
```

Add another component to an existing app (run in the folder with `spin.toml`):

```bash
spin add -t http-go api --accept-defaults --value http-path=/api/...
```

## Manifest essentials (`spin.toml`)

```toml
spin_manifest_version = 2

[application]
name = "my-app"
version = "0.1.0"

[variables]
api_token = { required = true }

[[trigger.http]]
route = "/..."              # "/..." = wildcard
component = "my-app"

[component.my-app]
source = "target/wasm32-wasip2/release/my_app.wasm"
allowed_outbound_hosts = ["https://api.example.com"]   # outbound HTTP/Redis/DB
key_value_stores = ["default"]                          # KV access
sqlite_databases = ["default"]                          # SQLite access
files = [{ source = "static", destination = "/" }]      # bundled files
[component.my-app.variables]
token = "{{ api_token }}"
[component.my-app.build]
command = "cargo build --target wasm32-wasip2 --release"
watch = ["src/**/*.rs", "Cargo.toml"]
```

Rules:
- `source` is relative to `spin.toml` and must match the real build output (renaming a Cargo package breaks it).
- `allowed_outbound_hosts` entries need scheme + host (port optional, default for scheme). `"*://*:*"` only for local experiments.
- Variables: declare at app level, expose per component, set at runtime with `SPIN_VARIABLE_API_TOKEN=... spin up` or `spin up --variable api_token=...`.
- Full reference: https://spinframework.dev/v4/manifest-reference

## Minimal handlers

**Rust** (`spin-sdk = "6"`, crate type `cdylib`; handlers are `async`):

```rust
use spin_sdk::http::{IntoResponse, Request, Response};
use spin_sdk::http_service;

#[http_service]
async fn handle(_req: Request) -> anyhow::Result<impl IntoResponse> {
    Ok(Response::builder()
        .status(200)
        .header("content-type", "text/plain")
        .body("Hello, Spin".to_string())?)
}
```

- KV: `let store = spin_sdk::key_value::Store::open_default().await?;` then `store.get/set(...).await`, or `get_json/set_json` with serde.
- Variables: `spin_sdk::variables::get("token").await?`
- Outbound HTTP: `spin_sdk::http::send(req).await?`
- Routing: use any `http`-compatible router (e.g. axum) only if needed.

**Go** (`github.com/spinframework/spin-go-sdk/v3`):

```go
package main

import (
	"fmt"
	"net/http"

	spinhttp "github.com/spinframework/spin-go-sdk/v3/http"
)

func init() {
	spinhttp.Handle(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "text/plain")
		fmt.Fprintln(w, "Hello, Spin")
	})
}

func main() {}
```

- Build: `go tool componentize-go build` (non-HTTP triggers also need `--world`, set by the template).
- Variables: `variables.Get("token")` from `.../v3/variables`; outbound: `spinhttp.Get/Send`.

For JS/TS and Python, follow the code generated by the template and the language guide.

## Other triggers

- **Redis:** `[application.trigger.redis] address = "redis://localhost:6379"` plus `[[trigger.redis]] channel = "..."`.
- **Cron:** a plugin trigger (`spin plugins install trigger-cron`, templates in `spinframework/spin-trigger-cron`). Verify against current docs before using.

## Run, debug, ship

```bash
spin build                          # runs each component's [build] command
spin up --listen 127.0.0.1:3000     # --direct-mounts for live static files
spin watch                          # rebuild + restart on change
spin registry push ghcr.io/<user>/my-app:0.1.0     # publish as OCI artifact
spin kube scaffold --from ghcr.io/<user>/my-app:0.1.0 > app.yaml   # SpinKube (kube plugin)
```

## Troubleshooting

| Symptom | Fix |
|---|---|
| `can't find crate for core` | `rustup target add wasm32-wasip2` |
| `Destination not allowed` | add the host to `allowed_outbound_hosts` |
| KV/SQLite access denied | add `key_value_stores` / `sqlite_databases` |
| `no provider resolved required variable` | set `SPIN_VARIABLE_<NAME>` or `--variable` |
| `no variable for "<component>"."x"` | add `x = "{{ app_var }}"` under `[component.<id>.variables]` |
| Wasm file not found | `source` path ≠ build output, or not built in `--release` |
| Host rejects the component | it only supports Spin 3 → recreate with a `-p2` template |

## Done when

- `spin build` succeeds with no warnings about missing sources
- `spin up` starts and `curl` returns the expected response
- `spin.toml` grants only the capabilities the code actually uses

## GitHub Copilot notes

- In agent mode, run commands in the terminal from the app folder; `spin up` blocks — run it as a background terminal, test with `curl`, then stop it.
- Never run `spin new`/`spin add` without `--accept-defaults`; the agent cannot answer interactive prompts.
- For the Copilot coding agent (cloud), Spin and the Wasm toolchain must be installed in `.github/workflows/copilot-setup-steps.yml` (e.g. `curl -fsSL https://spinframework.dev/downloads/install.sh | bash`, `rustup target add wasm32-wasip2`).
