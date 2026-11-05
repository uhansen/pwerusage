# SaveEyeEdgeListenerApp

The Raspberry Pi half of the SaveEye pipeline. This Spin MQTT app validates,
optionally filters, and normalizes local SaveEye telemetry, then publishes a
versioned envelope to the server broker. It has **no PostgreSQL dependency,
database credentials, or database network capability**.

Start with the [end-to-end deployment checklist](../README.md) for server/Pi
setup order, required inputs, acceptance checks and recovery.

```text
SaveEye on Wi-Fi
  -> Pi Mosquitto:1883 / saveeye/telemetry
  -> SaveEyeEdgeListenerApp (Docker, ARM64)
  -> server Mosquitto:1884 / pwrusage/sensors/saveeye/v1
  -> pwrusagesensorbackend (native Spin)
  -> server-local PostgreSQL
```

Set up the [backend](../../pwerusage/pwrusagesensorbackend/README.md) first. Both brokers
require credentials with separate publisher/subscriber ACLs. MQTT here is
authenticated but **not encrypted**: use only a trusted LAN, never public
port forwarding. The server publisher password is the only server credential
the Pi needs.

## Raspberry Pi 5 deployment

Use Raspberry Pi OS Lite **64-bit**, Docker Engine with Compose v2, and a
reserved LAN address. The listener is limited to 512 MiB and its broker to
128 MiB, suitable for a Pi 5 with 4 GB RAM. Keep the Pi clock synchronized
with NTP; its receipt timestamp controls latest-reading ordering.

### Build on the workstation

The edge now lives under `iotdevices/SaveEyeEdgeListenerApp`. From the
repository root, build it for ARM64. The build context must be the repository
root (`.`), because the image composes the reusable Wasm component from
`components/saveeye-telemetry`. The Dockerfile-specific ignore file excludes
secrets and unrelated apps. Composition tools are installed in the build
stage only; they are not needed on the Pi when loading a prebuilt image.

```bash
docker buildx build --platform linux/arm64 --load \
  -f iotdevices/SaveEyeEdgeListenerApp/Dockerfile \
  -t saveeye-edge-listener:pi .
docker image inspect saveeye-edge-listener:pi --format '{{.Os}}/{{.Architecture}}'
docker save -o /tmp/saveeye-edge-listener-pi.tar saveeye-edge-listener:pi

tar -czf /tmp/saveeye-edge-deploy.tar.gz \
  iotdevices/SaveEyeEdgeListenerApp/compose.yaml \
  iotdevices/SaveEyeEdgeListenerApp/.env.example \
  iotdevices/SaveEyeEdgeListenerApp/deploy/mosquitto \
  pwerusage/deploy/setup-secrets.py pwerusage/deploy/mosquitto
scp /tmp/saveeye-edge-listener-pi.tar /tmp/saveeye-edge-deploy.tar.gz pi@PI_ADDRESS:/tmp/
```

Replace `PI_ADDRESS` and the SSH user. The deployment archive deliberately
contains no secrets, backend files, or database configuration. Alternatively,
copy the repository to the Pi and build there with `docker compose build
listener`; the image build limits Cargo to two jobs.

### Configure on the Pi

```bash
docker load -i /tmp/saveeye-edge-listener-pi.tar
mkdir -p ~/pwrusage
tar -xzf /tmp/saveeye-edge-deploy.tar.gz -C ~/pwrusage
cd ~/pwrusage/iotdevices/SaveEyeEdgeListenerApp
cp .env.example .env
```

Edit `.env`:

| Setting | Meaning |
| --- | --- |
| `PI_LAN_IP` | Pi's actual LAN IPv4 address, not `0.0.0.0` |
| `MQTT_PORT` | Local SaveEye broker port, normally `1883` |
| `UPSTREAM_ADDRESS` | Server broker, e.g. `mqtt://192.168.1.20:1884`; no query, path, or credentials |
| `EDGE_ID` | Unique stable ID, 1-64 ASCII letters/digits or `_.-` |
| `SAVE_EYE_DEVICE_SERIAL` | Optional case-insensitive serial filter; empty accepts all |
| `SAVE_EYE_EDGE_IMAGE` | `saveeye-edge-listener:pi` for the transferred image |

Provision protected local credentials interactively:

```bash
python3 ../../pwerusage/deploy/setup-secrets.py edge
docker compose -p saveeye-edge config --quiet
docker compose -p saveeye-edge up -d --no-build
docker compose -p saveeye-edge ps
docker compose -p saveeye-edge logs --tail 50 listener
```

The helper prompts for local `saveeye-device` and `saveeye-listener`
passwords, then the server's `sensor-edge` publisher password. It creates a
mode-0700 secrets directory and mode-0600 files, and refuses to overwrite an
existing directory containing credentials. Do not copy `backend.env`, the
backend subscriber password, or database credentials to the Pi.

Configure **Local MQTT** in the SaveEye mobile app:

| Device setting | Value |
| --- | --- |
| Broker | Pi LAN IP, `mqtt://PI_ADDRESS` if the UI expects a URL |
| Port | `1883`, or the configured `MQTT_PORT` |
| Username | `saveeye-device` |
| Password | The local device password entered above |
| Topic | `saveeye/telemetry` |

For a Kamstrup meter, enter the grid operator's GPK60/GPK61 encryption key
only in the SaveEye app, never in this application's configuration.

Restrict the Pi's published MQTT port to the device/trusted subnet and the
server's MQTT port to the Pi, using Docker-aware host/router firewall rules.
Docker-published ports may bypass ordinary UFW rules. Do not expose either
broker or Docker administration to the internet.

## Behavior and operations

The [telemetry Wasm component](../../components/saveeye-telemetry/README.md)
preserves missing values, derives net import in watts and
cumulative energy in kWh, and wraps the reading with `schemaVersion: 1`,
`edgeId`, and edge-time `receivedAt`. See the backend README for the wire
contract and the database layout. The real payload fixture is in
`../../components/saveeye-telemetry/tests/fixtures/kamstrup-telemetry.json`.

Malformed input is logged and acknowledged. Each valid input gets a fresh
publisher client ID, preventing simultaneous invocations from disconnecting
each other. Outbound QoS 1 publishing has a five-second deadline. Success logs
start with `Forwarded:`: they mean **server broker acceptance**, not database
success. Failures are logged without passwords; there is no durable queue,
retained forwarding, or replay. A lost connection or stopped backend can lose
readings. Broker persistence does not change this policy.

The pinned MQTT trigger does not reconnect after its input broker restarts.
After Pi broker maintenance:

```bash
docker compose -p saveeye-edge up -d --wait mosquitto
docker compose -p saveeye-edge restart listener
```

After server broker maintenance, restart the native backend as documented
there. The edge opens a fresh outbound connection for each subsequent reading.
Docker restart policies recover exited processes, not a living trigger that
has lost its subscription. Disk logs are rotated; Spin duplicate file logging
is disabled with `--state-dir=`.

Keep protected backups of `.env` and `deploy/secrets` separately from source
control. To rotate credentials, coordinate the device, local subscriber, and
server publisher, regenerate files in a new protected directory, then switch
the appropriate deployment secret-directory setting and restart services.
Never use `docker compose down -v` on an existing installation unless its
broker data is intentionally disposable.

## Development and validation

The Docker runtime pins Spin 4.2.6 and MQTT trigger 0.9.6. For native development,
install that plugin for the user running Spin. A source build needs Rust 1.98+,
`wac-cli` 0.10.1 and `wasm-tools` 1.261.0:

```bash
spin plugins install --url https://github.com/spinframework/spin-trigger-plugins/releases/download/mqtt-v0.9.6/trigger-mqtt.json --yes
rustup target add wasm32-wasip2
cargo install wac-cli --version 0.10.1 --locked
cargo install wasm-tools --version 1.261.0 --locked --no-default-features --features component
spin build
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test --manifest-path ../../components/saveeye-telemetry/Cargo.toml
```

For a native edge process, set `SPIN_VARIABLE_EDGE_ID`,
`SPIN_VARIABLE_UPSTREAM_ADDRESS`, `SPIN_VARIABLE_UPSTREAM_OUTBOUND_HOST`
(the same bare endpoint), `SPIN_VARIABLE_UPSTREAM_USERNAME`, and
`SPIN_VARIABLE_UPSTREAM_PASSWORD`. Incoming broker variables are independent:
`SPIN_VARIABLE_MQTT_ADDRESS`, `SPIN_VARIABLE_MQTT_USERNAME`,
`SPIN_VARIABLE_MQTT_PASSWORD`, and optionally `SPIN_VARIABLE_DEVICE_SERIAL`.
Supply passwords through protected environment configuration, not argv.

Production `spin build` runs `build.sh`, builds both the MQTT adapter and
telemetry component for WASIp2, and uses `wac plug` to produce `composed.wasm`.
The manifest and runtime image load this composed artifact, not the unlinked
adapter under `target/`. Both apps generate imports from the component's
authoritative WIT; neither has a Rust path dependency on telemetry code.
The shared WASI skill script also builds the reusable component WASIp3-first,
independently of the production composition. The backend's integration suite
exercises this edge image through both brokers and native PostgreSQL persistence.
The Python credential helper and Mosquitto startup/config files remain
host-side under `pwerusage/deploy`, not inside the telemetry component.

## Rename and cutover

This replaces `SaveEyeListenerApp`; the old path, package, image and Wasm
artifact are not aliases. Do not start a second listener using the old
deployment inadvertently.

For an existing `pwerusage/SaveEyeEdgeListenerApp` deployment, stop its Compose
project before moving files. Preserve its `.env` and protected `deploy/secrets`
under the new `iotdevices/SaveEyeEdgeListenerApp` directory, and retain the same
Compose project name (`-p saveeye-edge` in this guide) to reuse its broker
volume. Keep the `pwerusage/deploy` helpers in the relative layout shown above.
Never leave a second edge process running from the old directory.

Back up existing database rows and the old deployment before cutover. Bring
up the backend and server broker, stop the old listener (preserving volumes),
then start this edge stack. If reusing an existing Pi broker/port, stop that
old broker before starting this stack and update the device's credentials.
Check both `Forwarded:` edge logs and actual backend rows. Keep the old image,
manifest and protected configuration for rollback; stop the new edge first
before restoring the old service. The additive backend migration preserves
old rows. Do not re-enable remote PostgreSQL access just to roll back.
