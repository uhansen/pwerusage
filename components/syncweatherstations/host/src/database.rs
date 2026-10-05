use anyhow::{Context, Result};
use tokio_postgres::{Client, NoTls};

use crate::bindings::pwrusage::weather_sync::{storage::PersistSummary, types::StationVersion};

const MIGRATION: &str = include_str!("../../migrations/001_station_versions.sql");
const ADVISORY_LOCK_ID: i64 = 8_327_304_551_219_103;

pub async fn connect(database_url: &str) -> Result<Client> {
    let (client, connection) = tokio_postgres::connect(database_url, NoTls)
        .await
        .context("connect to PostgreSQL")?;
    tokio::spawn(async move {
        if let Err(error) = connection.await {
            eprintln!("PostgreSQL connection error: {error}");
        }
    });
    Ok(client)
}

pub async fn migrate(client: &Client) -> Result<()> {
    client
        .batch_execute(MIGRATION)
        .await
        .context("apply station schema")
}

pub async fn persist(client: &mut Client, stations: &[StationVersion]) -> Result<PersistSummary> {
    let transaction = client.transaction().await.context("start transaction")?;
    transaction
        .query_one("SELECT pg_advisory_xact_lock($1)", &[&ADVISORY_LOCK_ID])
        .await
        .context("acquire sync lock")?;

    let mut inserted = 0_u64;
    let mut changed = 0_u64;
    let mut unchanged = 0_u64;

    let statement = transaction
        .prepare(
            r#"
            INSERT INTO station_versions (
                feature_id, station_id, name, country, owner, station_type, status,
                longitude, latitude, parameter_ids, region_id, wmo_country_code,
                wmo_station_id, station_height, barometer_height, source_created,
                source_updated, operation_from, operation_to, valid_from, valid_to,
                source_json, synced_at
            )
            VALUES (
                $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14,
                $15, $16::text::timestamptz, $17::text::timestamptz,
                $18::text::timestamptz, $19::text::timestamptz,
                $20::text::timestamptz, $21::text::timestamptz, $22::jsonb,
                now()
            )
            ON CONFLICT (feature_id) DO UPDATE SET
                station_id = EXCLUDED.station_id,
                name = EXCLUDED.name,
                country = EXCLUDED.country,
                owner = EXCLUDED.owner,
                station_type = EXCLUDED.station_type,
                status = EXCLUDED.status,
                longitude = EXCLUDED.longitude,
                latitude = EXCLUDED.latitude,
                parameter_ids = EXCLUDED.parameter_ids,
                region_id = EXCLUDED.region_id,
                wmo_country_code = EXCLUDED.wmo_country_code,
                wmo_station_id = EXCLUDED.wmo_station_id,
                station_height = EXCLUDED.station_height,
                barometer_height = EXCLUDED.barometer_height,
                source_created = EXCLUDED.source_created,
                source_updated = EXCLUDED.source_updated,
                operation_from = EXCLUDED.operation_from,
                operation_to = EXCLUDED.operation_to,
                valid_from = EXCLUDED.valid_from,
                valid_to = EXCLUDED.valid_to,
                source_json = EXCLUDED.source_json,
                synced_at = now()
            WHERE station_versions.source_json IS DISTINCT FROM EXCLUDED.source_json
            RETURNING (xmax = 0) AS inserted
            "#,
        )
        .await
        .context("prepare station upsert")?;

    for station in stations {
        let source_json: serde_json::Value =
            serde_json::from_str(&station.source_json).context("parse validated source JSON")?;
        let row = transaction
            .query_opt(
                &statement,
                &[
                    &station.feature_id,
                    &station.station_id,
                    &station.name,
                    &station.country,
                    &station.owner,
                    &station.station_type,
                    &station.status,
                    &station.longitude,
                    &station.latitude,
                    &station.parameter_ids,
                    &station.region_id,
                    &station.wmo_country_code,
                    &station.wmo_station_id,
                    &station.station_height,
                    &station.barometer_height,
                    &station.created,
                    &station.updated,
                    &station.operation_from,
                    &station.operation_to,
                    &station.valid_from,
                    &station.valid_to,
                    &source_json,
                ],
            )
            .await
            .with_context(|| format!("upsert station feature {}", station.feature_id))?;
        match row {
            Some(row) if row.get::<_, bool>("inserted") => inserted += 1,
            Some(_) => changed += 1,
            None => unchanged += 1,
        }
    }

    transaction.commit().await.context("commit station sync")?;
    Ok(PersistSummary {
        inserted,
        changed,
        unchanged,
    })
}
