# pwrusagesensorbackend

A **native Spin MQTT service** on the laptop/server. It validates SaveEye
envelopes forwarded by [SaveEyeEdgeListenerApp](../SaveEyeEdgeListenerApp/README.md)
and stores the latest reading per device in PostgreSQL. Only the server has
database credentials. This is initially a SaveEye backend, not a generic
sensor platform or HTTP API.

```text
Pi edge -> authenticated LAN MQTT:1884
        -> server Mosquitto -> native pwrusagesensorbackend
                            -> PostgreSQL at 127.0.0.1:5432
```

The supplied Compose file runs **only the server broker**, not the backend or
database. Port 1884 avoids the existing `saveeye-mosquitto` on 1883. Leave the
existing `postgres18` container, volume, and loopback port binding unchanged.
On another server, use that server's own local PostgreSQL.

## 1. Prepare PostgreSQL

Use a PostgreSQL administrator, not the application role. Back up an existing
installation before migrations. These commands are examples for the existing
container named `postgres18`; substitute its actual administrator if it is
not `postgres`.

For a **fresh** installation only, from this app directory:

```bash
docker exec postgres18 mkdir -p /tmp/saveeye-provision/deploy
docker cp deploy/provision.sql postgres18:/tmp/saveeye-provision/deploy/provision.sql
docker cp migrations postgres18:/tmp/saveeye-provision/migrations
docker exec -it postgres18 psql -X -U postgres -f /tmp/saveeye-provision/deploy/provision.sql
```

The script prompts for the `saveeye_listener` password without putting it in
argv or SQL history. It creates the `saveeye` database, a non-login
`saveeye_owner`, both migrations, and the restricted login role. Runtime
permissions are only CONNECT, schema USAGE, and SELECT/INSERT/UPDATE on
`saveeye.latest_readings`; no DELETE or DDL.

For an **existing database with migration 001 applied**, do not rerun
provisioning or drop the table. Apply only the additive migration:

```bash
docker cp migrations/002_edge_provenance.sql postgres18:/tmp/saveeye-002.sql
docker exec -it postgres18 psql -X -U postgres -d saveeye \
  -v ON_ERROR_STOP=1 -c 'SET ROLE saveeye_owner' -f /tmp/saveeye-002.sql
```

Migration 002 adds nullable `edge_id` and `backend_received_at`. Existing rows
and role credentials survive; the old table-level grants cover the new
columns. The handler never runs migrations.

Use a PostgreSQL URL shaped like
`postgresql://saveeye_listener:PERCENT_ENCODED_PASSWORD@127.0.0.1:5432/saveeye?sslmode=disable&connect_timeout=3&options=-c%20statement_timeout%3D5000`.
Enter the real URL only at the protected prompt below. Percent-encode special
characters in the password. `sslmode=disable` is appropriate only for the
existing non-TLS loopback database, never a remote connection.

If PostgreSQL is configured for TLS, use `sslmode=require` and a certificate
whose SAN matches the URL host. Spin validates the certificate chain and
hostname; provide a private CA through native `SSL_CERT_FILE` if needed.
Do not use the unsupported libpq `sslrootcert` URL option. The Pi never needs
this CA or connection URL. Database TLS configuration is optional and is not
changed by this deployment.

## 2. Configure the server broker

Use Docker Engine with Compose v2. From this app directory:

```bash
cp .env.example .env
# Edit SERVER_LAN_IP to this server's actual reserved LAN IPv4 address.
python3 ../deploy/setup-secrets.py server
docker compose -p pwrusagesensorbackend config --quiet
docker compose -p pwrusagesensorbackend up -d --wait
```

The helper creates `deploy/secrets` (0700) and its files (0600), and refuses
to overwrite credentials. It asks for:

| Credential | Used by |
| --- | --- |
| `sensor-edge` password | Pi publisher; share only this server password with the Pi |
| `sensor-backend` password | Native backend subscriber and broker health probe |
| Local PostgreSQL URL | Native backend only, stored in `backend.env` |

Passwords must contain at least 16 characters from letters, digits or
`_.~-`. The ACL permits `sensor-edge` to write only
`pwrusage/sensors/saveeye/v1`. `sensor-backend` can read that topic and the
broker-version health topic, but cannot publish. Anonymous access is disabled.

The broker binds loopback and `SERVER_LAN_IP`, never wildcard interfaces.
Reserve port 1884 (or change `SENSOR_MQTT_PORT`, the Pi upstream address, and
backend MQTT configuration together). Do not set `SERVER_LAN_IP=127.0.0.1`,
which would duplicate the loopback binding.

MQTT is password-authenticated but **unencrypted** on this trusted LAN.
Restrict the published server MQTT port to the Pi using Docker-aware firewall
rules; Docker may bypass ordinary UFW rules. No router port forwarding, public
MQTT, or LAN PostgreSQL exposure is needed.

## 3. Build and install native Spin

Build from the repository checkout, with sibling `../saveeye-telemetry`
available. Use Rust 1.98 or newer and Spin 4.2.6:

```bash
rustup target add wasm32-wasip2
spin build
```

Install the matching native Spin release on the server. For an x86-64 Linux
server (use `SPIN_ARCH=aarch64` for ARM64):

```bash
SPIN_ARCH=amd64
SPIN_DOWNLOAD=$(mktemp -d)
gh release download v4.2.6 --repo spinframework/spin \
  --pattern "spin-v4.2.6-linux-$SPIN_ARCH.tar.gz" \
  --pattern checksums-v4.2.6.txt --dir "$SPIN_DOWNLOAD"
(cd "$SPIN_DOWNLOAD" && sha256sum --check --ignore-missing checksums-v4.2.6.txt)
tar -xzf "$SPIN_DOWNLOAD/spin-v4.2.6-linux-$SPIN_ARCH.tar.gz" -C "$SPIN_DOWNLOAD"
sudo install -m 0755 "$SPIN_DOWNLOAD/spin" /usr/local/bin/spin
/usr/local/bin/spin --version
```

Create a dedicated service account and install the MQTT plugin **as that
account**, not just your interactive user. Skip `useradd` if already created:

```bash
sudo useradd --system --user-group --create-home \
  --home-dir /var/lib/pwrusagesensorbackend \
  --shell /usr/sbin/nologin pwrusage-sensor
sudo chmod 0750 /var/lib/pwrusagesensorbackend
sudo -u pwrusage-sensor env HOME=/var/lib/pwrusagesensorbackend \
  /usr/local/bin/spin plugins install --url \
  https://github.com/spinframework/spin-trigger-plugins/releases/download/mqtt-v0.9.6/trigger-mqtt.json --yes

sudo install -d -m 0755 /opt/pwrusagesensorbackend/target/wasm32-wasip2/release
sudo install -m 0644 spin.toml /opt/pwrusagesensorbackend/spin.toml
sudo install -m 0644 target/wasm32-wasip2/release/pwrusagesensorbackend.wasm \
  /opt/pwrusagesensorbackend/target/wasm32-wasip2/release/pwrusagesensorbackend.wasm
sudo install -d -o root -g pwrusage-sensor -m 0750 /etc/pwrusagesensorbackend
sudo install -o root -g pwrusage-sensor -m 0640 deploy/secrets/backend.env \
  /etc/pwrusagesensorbackend/backend.env
```

The generated environment has the backend MQTT password and database URL.
Defaults connect to `mqtt://127.0.0.1:1884` and allow only
`postgres://127.0.0.1:5432`. If changing endpoints, also add
`SPIN_VARIABLE_MQTT_ADDRESS` and/or `SPIN_VARIABLE_DB_OUTBOUND_HOST` to the
protected `/etc/pwrusagesensorbackend/backend.env`; the latter is a credential-
free capability endpoint and must match the database URL host and port.
For TLS with a private CA, add `SSL_CERT_FILE=/etc/pwrusagesensorbackend/ca.crt`
and install the readable CA certificate there. Do not put secrets in systemd
unit arguments or version control.

Install and start the service after the server broker/database are ready:

```bash
sudo install -m 0644 deploy/pwrusagesensorbackend.service /etc/systemd/system/
sudo systemd-analyze verify /etc/systemd/system/pwrusagesensorbackend.service
sudo systemctl daemon-reload
sudo systemctl enable --now pwrusagesensorbackend
sudo systemctl status pwrusagesensorbackend --no-pager
sudo journalctl -u pwrusagesensorbackend -n 50 --no-pager
```

The service uses a fixed working directory, unprivileged user, protected
configuration, writable `/var/lib/pwrusagesensorbackend` plugin/cache HOME,
and `--state-dir=` to avoid duplicate Spin disk logs. It restarts exited
processes and avoids hardening flags that prevent Wasmtime JIT execution.
Do not replace `/usr/local/bin/spin` with a binary inside a protected home
directory without adapting the unit.

For temporary foreground development, load only the locally generated,
trusted environment file with `set -a; . ./deploy/secrets/backend.env; set +a`,
then run `spin up --state-dir=`. Stop that process before starting the service;
two backend subscribers would both process each message.

## Contract and storage

Shared code lives in `../saveeye-telemetry`. The wire format is:

```json
{
  "schemaVersion": 1,
  "edgeId": "pi-saveeye-1",
  "reading": {
    "receivedAt": "2026-01-01T12:00:00Z",
    "meterKind": "unknown"
  }
}
```

The minimal example is valid; real readings include the known SaveEye payload
fields and optional `netConsumptionW`, `activeTotalConsumptionKwh`, and
`activeTotalProductionKwh`. Missing measurements remain missing, never zero.
Unsupported versions, invalid/missing edge IDs or timestamps, malformed JSON,
and inconsistent meter-kind/derived values are logged and acknowledged without
writing. `edgeId` is 1-64 ASCII letters/digits or `_.-`. Broker credentials
authenticate the publisher account, **not a cryptographic binding of edgeId**.

`saveeye.latest_readings` retains one JSONB reading per device serial. The
original `"unknown"` fallback for absent device serials is preserved; edge ID
does not become part of the primary key. Two edges forwarding the same serial
share the same row. Only a strictly newer **edge receipt timestamp** updates
it. Equal-time duplicates and older arrivals are no-ops, including metadata.
`edge_id` records provenance and `backend_received_at` records server receipt
time separately. Keep edge clocks synchronized to avoid future timestamps
blocking subsequent readings.

Verify actual persistence as an administrator:

```bash
docker exec -it postgres18 psql -X -U postgres -d saveeye -c \
  'SELECT device_serial, received_at, edge_id, backend_received_at FROM saveeye.latest_readings;'
```

There is **no durable forwarding queue, history, retained publication, or
offline replay**. QoS 1 means broker acceptance, not successful database
commit. Database failures are logged and later messages can succeed; failed
writes are not replayed. A backend outage may lose readings even while the
edge reports successful forwarding.

## Maintenance, backup and cutover

The pinned MQTT trigger does not resubscribe after its broker restarts.
After server broker maintenance, run:

```bash
docker compose -p pwrusagesensorbackend up -d --wait
sudo systemctl restart pwrusagesensorbackend
```

For Pi broker restarts, restart the edge listener separately. Systemd/Docker
restart policies do not detect a living trigger with a lost subscription.

Before upgrading, keep the old app/Wasm/image/configuration and make a
protected database backup:

```bash
umask 077
docker exec postgres18 pg_dump -U postgres -d saveeye -Fc > saveeye.dump
```

Role definitions/passwords are not included in this dump: retain protected
role provisioning/configuration separately. Test restores into a separate
database, never by dropping live readings:

```bash
docker exec postgres18 createdb -U postgres -O saveeye_owner saveeye_restore
docker exec -i postgres18 pg_restore -U postgres -d saveeye_restore --exit-on-error < saveeye.dump
docker exec postgres18 psql -X -U postgres -d saveeye_restore \
  -c 'SELECT count(*) FROM saveeye.latest_readings;'
```

Back up broker/backend secrets separately with restricted access. For password
rotation, coordinate publisher/subscriber clients and broker credentials;
the helper intentionally refuses to overwrite live files.

Apply migration 002, start the new server broker and backend, then stop the
old `SaveEyeListenerApp` and replace it with the renamed Pi edge deployment.
Preserve old broker volumes and avoid overlapping port bindings. Verify a
physical reading through edge logs and a new database timestamp before
removing the old deployment. Rollback restores the old app/configuration
after stopping the new edge; additive metadata columns can remain. No
existing container or firewall needs automatic modification.

## Tests

From this directory:

```bash
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test --manifest-path ../saveeye-telemetry/Cargo.toml
spin build
docker build -f ../SaveEyeEdgeListenerApp/Dockerfile \
  -t saveeye-edge-listener:0.1.0 ..
SPIN_BIN=/usr/local/bin/spin python3 tests/integration.py
```

The integration runner creates uniquely named disposable brokers and a TLS
PostgreSQL container, runs the **real native backend**, and deletes only its
own resources. It checks forwarding, normalization, filtering, ACL positive
controls and denials, invalid envelopes, existing-row migration, stale/
duplicate ordering, runtime database permissions, outages, certificate
rejection, bounded edge failures, restart recovery, and backup/restore.
It never uses the real SaveEye device or existing database. To test ARM64,
build the edge image with `--platform linux/arm64 --load` as in the edge
README and set `SAVE_EYE_TEST_IMAGE=saveeye-edge-listener:pi`; workstation
execution requires Docker/binfmt ARM64 emulation.
