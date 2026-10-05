# syncweatherstations

Rust WASI component that synchronizes DMI meteorological station metadata into
PostgreSQL. The component owns pagination, parsing, validation, and retry
decisions. A native Wasmtime host supplies restricted HTTPS and transactional
PostgreSQL capabilities.

## Database setup

Create a dedicated login and database once in the existing local PostgreSQL
container:

```bash
docker exec postgres18 psql -U postgres -d postgres -v ON_ERROR_STOP=1 \
  -c "CREATE ROLE weatherstations_sync LOGIN"
docker exec postgres18 createdb -U postgres \
  -O weatherstations_sync weatherstations
```

This laptop's container accepts trusted local TCP connections. If its
authentication changes, configure the role password outside the repository.
Supply the connection externally and do not commit credentials:

```bash
export DATABASE_URL='postgresql://weatherstations_sync@127.0.0.1:5432/weatherstations'
```

## Build and run

From this directory:

```bash
../../.github/skills/wasi-component-build/scripts/build.sh
cargo build --manifest-path host/Cargo.toml
../../.github/skills/wasi-component-build/scripts/run.sh \
  --host host/target/debug/syncweatherstations-host \
  target/wasm32-wasip3/debug/syncweatherstations.wasm
```

Use the artifact path printed by the build script if it falls back to WASIp2.
The host links both WASIp3 and WASIp2 and applies the database migration before
each synchronization.

## Queries

Current metadata includes active and inactive stations:

```sql
SELECT station_id, name, station_type, status
FROM current_weather_stations
ORDER BY station_id;
```

Stations currently marked active and measuring dry-bulb temperature:

```sql
SELECT station_id, name, station_type
FROM current_weather_stations
WHERE status = 'Active'
  AND 'temp_dry' = ANY(parameter_ids)
ORDER BY station_id;
```

Filter by `country = 'DNK'` for Denmark or `station_type = 'Synop'` for Synop
stations. The DMI bounding box `7,54,16,58` is only an approximation of Denmark.
