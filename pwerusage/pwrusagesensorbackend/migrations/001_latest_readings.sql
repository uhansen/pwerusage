BEGIN;

CREATE SCHEMA IF NOT EXISTS saveeye;
CREATE TABLE IF NOT EXISTS saveeye.latest_readings (
    device_serial text PRIMARY KEY,
    received_at timestamptz NOT NULL,
    reading jsonb NOT NULL CHECK (jsonb_typeof(reading) = 'object')
);

CREATE INDEX IF NOT EXISTS latest_readings_received_at
    ON saveeye.latest_readings (received_at DESC);

COMMIT;
