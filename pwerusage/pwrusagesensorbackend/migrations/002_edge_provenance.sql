BEGIN;
ALTER TABLE saveeye.latest_readings
    ADD COLUMN IF NOT EXISTS edge_id text,
    ADD COLUMN IF NOT EXISTS backend_received_at timestamptz;
COMMIT;
