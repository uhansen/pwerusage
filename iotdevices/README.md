# IoT device deployment

The SaveEye deployment consists of a Raspberry Pi edge and a laptop/server
backend. Deploy the server first, then the Pi. Commands in the linked guides
are deployment instructions; running repository tests does not install these
services or configure a physical Pi.

```text
SaveEye device on Wi-Fi
  -> Raspberry Pi: Mosquitto + SaveEyeEdgeListenerApp (Docker Compose)
  -> server: Mosquitto (Docker Compose) + pwrusagesensorbackend (native Spin)
  -> server-local PostgreSQL
```

## Preparation

Use Raspberry Pi OS Lite **64-bit** on the Pi 5. Install Docker Engine,
Compose v2 and Python 3 on both machines; enable Docker at boot. The build
workstation additionally needs Docker Buildx to produce the Pi's ARM64 image.
The Pi needs neither Rust nor Spin installed on its host when loading the
prebuilt image.

Reserve LAN addresses for both machines, enable clock synchronization, and
confirm they can reach each other. Decide the following before provisioning:

| Input | Default/example |
| --- | --- |
| Pi LAN IP and SSH login | Your Pi's reserved address and configured user |
| Server LAN IP | Your laptop/server's reserved address |
| Pi device-facing MQTT port/topic | `1883` / `saveeye/telemetry` |
| Server edge-facing MQTT port/topic | `1884` / `pwrusage/sensors/saveeye/v1` |
| Edge identifier | A unique stable value such as `pi-saveeye-1` |
| PostgreSQL | Server-local `127.0.0.1:5432`, database `saveeye` |

MQTT is authenticated but **unencrypted**, for a trusted LAN only. Restrict
the server MQTT port to the Pi and the Pi MQTT port to the device/trusted
subnet using Docker-aware firewall rules. Do not expose MQTT, PostgreSQL,
or Docker administration to the internet.

## Deployment order

1. Follow the [server runbook](../pwerusage/pwrusagesensorbackend/README.md):
   prepare or upgrade PostgreSQL, generate server credentials, start its
   broker, and install the native Spin backend with its systemd service.
   Existing `postgres18` remains loopback-only; do not recreate its container
   or volume. Keep the existing development broker on port 1883 separate.
2. Follow the [Pi runbook](SaveEyeEdgeListenerApp/README.md): build/save the
   ARM64 image on the workstation, transfer it with the deploy-only archive,
   configure the Pi, generate credentials, and start its Compose stack.
   Transfer only the server's `sensor-edge` publisher password to the Pi,
   never the backend MQTT password or database URL.
3. Configure Local MQTT in the SaveEye mobile app to use the Pi's address,
   the local `saveeye-device` credentials, and `saveeye/telemetry`. Wait for
   a fresh physical reading and verify every hop below.

The Pi archive preserves this runtime layout:

```text
~/pwrusage/
  iotdevices/SaveEyeEdgeListenerApp/
    compose.yaml
    .env
    deploy/mosquitto/acl
    deploy/secrets/
  pwerusage/deploy/
    setup-secrets.py
    mosquitto/
```

Do not flatten these directories: Compose mounts and the credential generator
use their relative paths. The reusable
[telemetry Wasm component](../components/saveeye-telemetry/README.md) is already
composed into the image and backend `composed.wasm`; it does not need a third
running service. Source builds require the component source plus the pinned
composition tools; runtime machines receiving built artifacts do not.

## End-to-end acceptance

On the Pi, from `~/pwrusage/iotdevices/SaveEyeEdgeListenerApp`:

```bash
docker compose -p saveeye-edge ps
docker compose -p saveeye-edge logs --tail 50 listener
```

Both containers should be running and the broker healthy. A fresh physical
reading should produce `Forwarded:` in the listener logs. This confirms
server broker acceptance, **not** a successful database write.

On the server, from the checkout's `pwerusage/pwrusagesensorbackend`:

```bash
docker compose -p pwrusagesensorbackend ps
sudo systemctl status pwrusagesensorbackend --no-pager
sudo journalctl -u pwrusagesensorbackend -n 50 --no-pager
docker exec -it postgres18 psql -X -U postgres -d saveeye -c \
  'SELECT device_serial, received_at, edge_id, backend_received_at FROM saveeye.latest_readings;'
```

Confirm the physical device serial, expected edge ID and a recent timestamp.
Wait for another device publication and confirm the row advances. Substitute
your database container/administrator if different from the examples.

During a planned maintenance window, reboot the Pi and verify both services
return and new readings arrive. Check resource use with
`docker stats --no-stream`; native Pi reboot, Wi-Fi and resource checks are
still required even if workstation tests passed under ARM64 emulation.

## Recovery and maintenance

| Symptom | Check/action |
| --- | --- |
| No Pi readings | Check SaveEye Wi-Fi, Local MQTT address, credentials/topic, serial filter and Pi logs |
| Pi cannot forward | Check server address/port, LAN firewall, matching publisher password, broker health and ACL |
| Edge forwards but no database update | Check native backend subscription/logs, database credentials, migrations, TLS and Pi clock |
| Broker was restarted | Explicitly restart its MQTT-trigger listener as shown below |
| Credentials helper refuses to run | It deliberately refuses to overwrite an existing secrets directory; preserve it and follow coordinated rotation in the runbooks |

After restarting the Pi broker, run on the Pi:

```bash
docker compose -p saveeye-edge up -d --wait mosquitto
docker compose -p saveeye-edge restart listener
```

After restarting the server broker, run on the server:

```bash
docker compose -p pwrusagesensorbackend up -d --wait
sudo systemctl restart pwrusagesensorbackend
```

The pinned MQTT trigger does not reconnect automatically after losing its
input broker, even when its process is still running. There is no durable
forwarding queue or offline replay; outages can lose readings, and recovery
is verified using subsequent publications.

Back up the database, protected credentials, non-secret settings, and previous
image/app artifacts before upgrades. The server runbook covers additive
migrations and restore testing; the Pi runbook covers the directory move,
cutover and rollback. Preserve Compose project names and broker volumes.
Never use `docker compose down -v` to perform an ordinary upgrade.
