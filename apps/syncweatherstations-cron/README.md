# syncweatherstations-cron

Spin cron app that runs once per week and synchronizes DMI weather
stations into the local Docker PostgreSQL database.

## What it does

- uses the existing `components/syncweatherstations` WebAssembly component for
  pagination, validation and change detection
- uses a small Spin adapter component for outbound HTTPS and PostgreSQL writes
- composes the cron handler, adapter and sync component into `composed.wasm`

## Prerequisites

- Spin with the `trigger-cron` plugin installed
- the local `postgres18` Docker container running on `127.0.0.1:5432`
- the `weatherstations` database and `weatherstations_sync` role created as
  described in `components/syncweatherstations/README.md`

## Build

From this directory:

```bash
./build.sh
```

or:

```bash
spin build
```

## Run

```bash
SPIN_VARIABLE_DB_URL='postgresql://weatherstations_sync@127.0.0.1:5432/weatherstations' \
  spin build --up
```

For faster verification, temporarily change the cron expression in `spin.toml`
to a short interval such as `0/15 * * * * *`, run the app, then restore the
weekly schedule.

## Local Docker deployment with OpenTelemetry

`docker-compose.yml` uses:

- `weatherstations-db`: PostgreSQL 18 with a local named volume
- `dmi-proxy`: local reverse proxy that fetches DMI over HTTPS for the Spin app
- `syncweatherstations-cron`: Spin 4.2.3 with the `trigger-cron` plugin
- `otel-collector`: OTLP receiver plus debug export
- `jaeger`: trace UI at <http://localhost:16686>

Build the composed Wasm first, then start the stack:

```bash
spin build
docker compose up --build -d
```

The Docker deployment uses `spin.compose.toml`, which keeps the committed weekly
schedule in `spin.toml` but runs every 30 seconds locally so traces and database
writes can be investigated without waiting a week.

Useful commands:

```bash
docker compose logs -f syncweatherstations-cron
docker compose logs -f dmi-proxy
docker compose logs -f otel-collector
docker compose exec weatherstations-db \
  psql -U weatherstations_sync -d weatherstations -c 'SELECT COUNT(*) FROM station_versions;'
docker compose down
```
