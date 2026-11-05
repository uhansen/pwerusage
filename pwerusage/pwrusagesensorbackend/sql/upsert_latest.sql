INSERT INTO saveeye.latest_readings (device_serial, received_at, reading, edge_id, backend_received_at)
VALUES ($1, $2::text::timestamptz, $3, $4, clock_timestamp())
ON CONFLICT (device_serial) DO UPDATE
SET received_at = EXCLUDED.received_at,
    reading = EXCLUDED.reading,
    edge_id = EXCLUDED.edge_id,
    backend_received_at = EXCLUDED.backend_received_at
WHERE EXCLUDED.received_at > saveeye.latest_readings.received_at
