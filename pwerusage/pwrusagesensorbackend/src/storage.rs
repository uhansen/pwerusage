use anyhow::{Context, Result, anyhow};
use spin_sdk::{
    pg::{Connection, ParameterValue},
    variables,
};

use crate::telemetry::pwrusage::saveeye::processor::ValidatedReading;

const UPSERT: &str = include_str!("../sql/upsert_latest.sql");

pub async fn persist(reading: &ValidatedReading) -> Result<()> {
    let database_url = variables::get("db_url")
        .await
        .context("read required db_url variable")?;
    // Connection errors may echo a malformed URL containing credentials.
    let connection = Connection::open(database_url).await.map_err(|_| {
        anyhow!(
            "open SaveEye PostgreSQL connection failed; check connectivity, credentials, TLS and db_outbound_host"
        )
    })?;
    connection
        .execute(
            UPSERT,
            vec![
                ParameterValue::Str(reading.device_serial.clone()),
                ParameterValue::Str(reading.received_at.clone()),
                ParameterValue::Jsonb(reading.reading_json.clone()),
                ParameterValue::Str(reading.edge_id.clone()),
            ],
        )
        .await
        .context("upsert latest SaveEye reading")?;
    Ok(())
}
