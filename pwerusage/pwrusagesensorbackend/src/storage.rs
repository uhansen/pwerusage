use anyhow::{Context, Result, anyhow};
use spin_sdk::{
    pg::{Connection, ParameterValue},
    variables,
};

use saveeye_telemetry::Envelope;

const UPSERT: &str = include_str!("../sql/upsert_latest.sql");

pub async fn persist(envelope: &Envelope) -> Result<()> {
    let reading = &envelope.reading;
    let database_url = variables::get("db_url")
        .await
        .context("read required db_url variable")?;
    let body = serde_json::to_vec(reading).context("encode SaveEye reading")?;
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
                ParameterValue::Str(reading.telemetry.device_serial().to_owned()),
                ParameterValue::Str(reading.received_at.clone()),
                ParameterValue::Jsonb(body),
                ParameterValue::Str(envelope.edge_id.clone()),
            ],
        )
        .await
        .context("upsert latest SaveEye reading")?;
    Ok(())
}
