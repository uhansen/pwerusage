CREATE TABLE IF NOT EXISTS station_versions (
    feature_id text PRIMARY KEY,
    station_id text NOT NULL,
    name text NOT NULL,
    country text NOT NULL,
    owner text NOT NULL,
    station_type text NOT NULL,
    status text NOT NULL CHECK (status IN ('Active', 'Inactive')),
    longitude double precision NOT NULL CHECK (longitude BETWEEN -180 AND 180),
    latitude double precision NOT NULL CHECK (latitude BETWEEN -90 AND 90),
    parameter_ids text[] NOT NULL,
    region_id text,
    wmo_country_code text,
    wmo_station_id text,
    station_height double precision,
    barometer_height double precision,
    source_created timestamptz NOT NULL,
    source_updated timestamptz,
    operation_from timestamptz NOT NULL,
    operation_to timestamptz,
    valid_from timestamptz NOT NULL,
    valid_to timestamptz,
    source_json jsonb NOT NULL,
    synced_at timestamptz NOT NULL DEFAULT now(),
    CHECK (operation_to IS NULL OR operation_to >= operation_from),
    CHECK (valid_to IS NULL OR valid_to >= valid_from)
);

CREATE INDEX IF NOT EXISTS station_versions_station_id_idx
    ON station_versions (station_id);
CREATE INDEX IF NOT EXISTS station_versions_validity_idx
    ON station_versions (valid_from, valid_to);
CREATE INDEX IF NOT EXISTS station_versions_status_idx
    ON station_versions (status);
CREATE INDEX IF NOT EXISTS station_versions_parameter_ids_idx
    ON station_versions USING gin (parameter_ids);

CREATE OR REPLACE VIEW current_weather_stations AS
SELECT DISTINCT ON (station_id)
    feature_id,
    station_id,
    name,
    country,
    owner,
    station_type,
    status,
    longitude,
    latitude,
    parameter_ids,
    region_id,
    wmo_country_code,
    wmo_station_id,
    station_height,
    barometer_height,
    operation_from,
    operation_to,
    valid_from,
    valid_to
FROM station_versions
WHERE valid_from <= now()
  AND (valid_to IS NULL OR now() < valid_to)
ORDER BY station_id, valid_from DESC, feature_id DESC;
