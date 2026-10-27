# SaveEyeListenerApp

A Spin application that listens to [SaveEye+ for Kamstrup](https://saveeye.dk/saveeye-for-kamstrup)
telemetry over MQTT, validates and normalizes each reading, and stores it in Spin's
key-value store.

The payload model and derived values are inspired by the official
[SaveEye Home Assistant integration](https://github.com/saveeye/SaveEye-HomeAssistant)
and its [local MQTT guide](https://github.com/saveeye/SaveEye-HA-Guide).

```
SaveEye+ ──MQTT──▶ broker (Mosquitto, EMQX, ...) ──MQTT trigger──▶ save-eye-listener-app ──▶ key-value store
```

## What it does

For every message on `saveeye/telemetry` the component:

1. Decodes the JSON telemetry payload.
2. Optionally filters on `saveeyeDeviceSerialNumber`.
3. Derives net power (import minus export) and kWh totals.
4. Writes the reading to the `default` key-value store:

   | Key | Contents |
   | --- | --- |
   | `saveeye:latest` | most recent reading from any device |
   | `saveeye:devices:<serial>` | most recent reading for one device |

5. Logs a one-line summary, for example:

   ```
   SaveEye J85LH2H4 [kamstrup] meter 68412345: importing 812 W, 18423.30 kWh consumed, 2150.40 kWh produced, 231 V
   ```

Stored readings are the original SaveEye JSON plus a few derived fields:

```json
{
  "receivedAt": "2026-10-27T16:31:56+00:00",
  "meterKind": "kamstrup",
  "netConsumptionW": 812.0,
  "activeTotalConsumptionKwh": 18423.3,
  "activeTotalProductionKwh": 2150.4,
  "saveeyeDeviceSerialNumber": "J85LH2H4",
  "meterType": "Kamstrup Omnipower",
  "meterSerialNumber": "68412345",
  "activeActualConsumption": { "total": 812.0 },
  "activeTotalConsumption": { "total": 18423300.0 },
  "...": "all other fields from the SaveEye schema"
}
```

Malformed messages and non-UTF-8 payloads are logged and acknowledged, so a bad
message is not redelivered forever. Not every meter reports every field; for
example a Kamstrup meter may report only `total` values and no per-phase values.
See `tests/fixtures/kamstrup-telemetry.json` for a complete sample.

## Prerequisites

* Spin 4.x (`spin --version`)
* Rust with the `wasm32-wasip2` target: `rustup target add wasm32-wasip2`
* The MQTT trigger plugin and template:

  ```bash
  spin plugins install trigger-mqtt --yes
  spin templates install --git https://github.com/spinframework/spin-trigger-plugins --update
  ```

* An MQTT broker reachable by both the SaveEye device and Spin.

## Configure the SaveEye device

In the SaveEye app, open the device settings and enable **Local MQTT**:

| Setting | Value |
| --- | --- |
| Broker URL | `mqtt://<broker-host>` (for example `mqtt://homeassistant.local`) |
| Port | `1883` (or your broker's port) |
| Username / password | credentials for the broker |
| Topic | `saveeye/telemetry` |

The device publishes to the topic configured here; keep `mqtt_topic` in
`spin.toml` in sync.

> For a Kamstrup Omnipower meter, SaveEye requires the GPK60/GPK61 encryption
> key from your grid operator. That key is entered in the SaveEye app, not in
> this application.

## Configure and run

The broker connection and filter are Spin application variables with sensible
defaults:

| Variable | Default | Purpose |
| --- | --- | --- |
| `mqtt_address` | `mqtt://localhost:1883` | broker address |
| `mqtt_username` | *(empty)* | broker username |
| `mqtt_password` | *(empty)* | broker password |
| `mqtt_topic` | `saveeye/telemetry` | topic published by the SaveEye device |
| `device_serial` | *(empty)* | only accept this device serial; empty accepts all |

Run with defaults:

```bash
spin build
spin up
```

Run against your broker and a specific device:

```bash
spin up \
  --variable mqtt_address=mqtt://192.168.1.10:1883 \
  --variable mqtt_username=saveeye \
  --variable mqtt_password=secret \
  --variable device_serial=J85LH2H4
```

Spin stores key-value data in `.spin/sqlite_key_value.db` when running locally.

## Verify

With a local broker (for example `docker run -d -p 1883:1883 eclipse-mosquitto:2`
with anonymous access enabled) publish the sample payload:

```bash
mosquitto_pub -h localhost -t saveeye/telemetry \
  -f tests/fixtures/kamstrup-telemetry.json
```

The Spin log shows the summary line, and the reading can be inspected with:

```bash
sqlite3 .spin/sqlite_key_value.db \
  "select key, value from spin_key_value where key like 'saveeye:%';"
```

## Tests

The telemetry model and derived values have unit tests that run on the host:

```bash
cargo test
```

## Notes

* The MQTT trigger plugin is receive-only. This app stores readings locally;
  downstream components can read them from the key-value store.
* Each MQTT message triggers a fresh component instance, so state is kept in the
  key-value store rather than in memory.
* The `device_serial` filter is case-insensitive.
