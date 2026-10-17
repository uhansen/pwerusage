use std::{thread, time::Duration};

use anyhow::{anyhow, Context, Result};
use serde_json::Value;
use spin_sdk::{
    http::{self, body::IncomingBodyExt, EmptyBody, Request},
    pg::{Connection, ParameterValue},
    variables,
};
use url::Url;

const MIGRATION: &str =
    include_str!("../../../../components/syncweatherstations/migrations/001_station_versions.sql");
const ADVISORY_LOCK_ID: i64 = 8_327_304_551_219_103;
const UPSERT_STATION_SQL: &str = r#"
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
"#;

wit_bindgen::generate!({
    inline: r#"
        package pwrusage:weather-sync@0.1.0;

        interface types {
            record station-version {
                feature-id: string,
                station-id: string,
                name: string,
                country: string,
                owner: string,
                station-type: string,
                status: string,
                longitude: f64,
                latitude: f64,
                parameter-ids: list<string>,
                region-id: option<string>,
                wmo-country-code: option<string>,
                wmo-station-id: option<string>,
                station-height: option<f64>,
                barometer-height: option<f64>,
                created: string,
                updated: option<string>,
                operation-from: string,
                operation-to: option<string>,
                valid-from: string,
                valid-to: option<string>,
                source-json: string,
            }

            record persist-summary {
                inserted: u64,
                changed: u64,
                unchanged: u64,
            }
        }

        interface transport {
            record response {
                status: u16,
                body: string,
                retry-after-ms: option<u64>,
            }

            get: async func(url: string) -> result<response, string>;
            sleep-ms: async func(milliseconds: u64);
        }

        interface storage {
            use types.{station-version, persist-summary};

            persist: async func(stations: list<station-version>) -> result<persist-summary, string>;
        }

        world adapter {
            export transport;
            export storage;
        }
    "#,
});

struct Component;

impl exports::pwrusage::weather_sync::transport::Guest for Component {
    async fn get(
        url: String,
    ) -> Result<exports::pwrusage::weather_sync::transport::Response, String> {
        get_impl(&url).await.map_err(|error| format!("{error:#}"))
    }

    async fn sleep_ms(milliseconds: u64) {
        thread::sleep(Duration::from_millis(milliseconds));
    }
}

impl exports::pwrusage::weather_sync::storage::Guest for Component {
    async fn persist(
        stations: Vec<pwrusage::weather_sync::types::StationVersion>,
    ) -> Result<pwrusage::weather_sync::types::PersistSummary, String> {
        persist_impl(&stations)
            .await
            .map_err(|error| format!("{error:#}"))
    }
}

#[cfg(target_family = "wasm")]
export!(Component);

async fn get_impl(url: &str) -> Result<exports::pwrusage::weather_sync::transport::Response> {
    let request_url = proxied_url(url).await?;
    let request = Request::builder()
        .method("GET")
        .uri(&request_url)
        .body(EmptyBody::new())
        .context("build outbound HTTP request")?;
    let response = http::send(request)
        .await
        .context("send outbound HTTP request")?;
    let status = response.status().as_u16();
    let retry_after_ms = response
        .headers()
        .get("retry-after")
        .and_then(|value| value.to_str().ok())
        .and_then(parse_retry_after_ms);
    let body = response
        .into_body()
        .bytes()
        .await
        .context("read outbound HTTP body")?;
    let body = String::from_utf8(body.to_vec()).context("decode outbound HTTP body as UTF-8")?;

    Ok(exports::pwrusage::weather_sync::transport::Response {
        status,
        body,
        retry_after_ms,
    })
}

async fn proxied_url(url: &str) -> Result<String> {
    let proxy_base_url = match variables::get("dmi_proxy_base_url").await {
        Ok(value) if !value.trim().is_empty() => value,
        Ok(_) | Err(_) => return Ok(url.to_string()),
    };

    let upstream = Url::parse(url).context("parse upstream DMI URL")?;
    let proxy_base = Url::parse(&proxy_base_url).context("parse dmi_proxy_base_url")?;
    if proxy_base.query().is_some() || proxy_base.fragment().is_some() {
        anyhow::bail!("dmi_proxy_base_url must not include a query string or fragment");
    }

    let mut proxied = proxy_base;
    proxied.set_path(upstream.path());
    proxied.set_query(upstream.query());
    Ok(proxied.into())
}

async fn persist_impl(
    stations: &[pwrusage::weather_sync::types::StationVersion],
) -> Result<pwrusage::weather_sync::types::PersistSummary> {
    let database_url = variables::get("db_url")
        .await
        .context("read db_url variable")?;
    let connection = Connection::open(database_url)
        .await
        .context("open PostgreSQL connection")?;
    apply_migration(&connection).await?;
    begin_transaction(&connection).await?;

    let result = persist_in_transaction(&connection, stations).await;
    match result {
        Ok(summary) => {
            execute_simple(&connection, "COMMIT")
                .await
                .context("commit station sync")?;
            Ok(summary)
        }
        Err(error) => {
            if let Err(rollback_error) = execute_simple(&connection, "ROLLBACK").await {
                return Err(anyhow!("{error:#}; rollback failed: {rollback_error:#}"));
            }
            Err(error)
        }
    }
}

async fn apply_migration(connection: &Connection) -> Result<()> {
    for statement in MIGRATION
        .split(';')
        .map(str::trim)
        .filter(|statement| !statement.is_empty())
    {
        execute_simple(connection, statement)
            .await
            .with_context(|| format!("apply migration statement: {statement}"))?;
    }
    Ok(())
}

async fn begin_transaction(connection: &Connection) -> Result<()> {
    execute_simple(connection, "BEGIN")
        .await
        .context("start transaction")?;
    let lock_rows = connection
        .query(
            "SELECT pg_advisory_xact_lock($1)",
            vec![ParameterValue::Int64(ADVISORY_LOCK_ID)],
        )
        .await
        .context("acquire sync lock")?;
    lock_rows
        .collect()
        .await
        .context("finish advisory lock query")?;
    Ok(())
}

async fn persist_in_transaction(
    connection: &Connection,
    stations: &[pwrusage::weather_sync::types::StationVersion],
) -> Result<pwrusage::weather_sync::types::PersistSummary> {
    let mut inserted = 0_u64;
    let mut changed = 0_u64;
    let mut unchanged = 0_u64;

    for station in stations {
        let source_json: Value =
            serde_json::from_str(&station.source_json).context("parse validated source JSON")?;
        let params = station_params(station, source_json)?;
        let result = connection
            .query(UPSERT_STATION_SQL, params)
            .await
            .with_context(|| format!("upsert station feature {}", station.feature_id))?;
        let rows = result
            .collect()
            .await
            .context("collect station upsert row")?;
        match rows.as_slice() {
            [] => unchanged += 1,
            [row]
                if row
                    .get::<bool>("inserted")
                    .context("decode inserted flag from station upsert row")? =>
            {
                inserted += 1;
            }
            [_] => changed += 1,
            _ => {
                return Err(anyhow!(
                    "upsert station feature {} returned multiple rows",
                    station.feature_id
                ));
            }
        }
    }

    Ok(pwrusage::weather_sync::types::PersistSummary {
        inserted,
        changed,
        unchanged,
    })
}

fn station_params(
    station: &pwrusage::weather_sync::types::StationVersion,
    source_json: Value,
) -> Result<Vec<ParameterValue>> {
    let source_json =
        serde_json::to_vec(&source_json).context("encode source JSON for PostgreSQL")?;
    Ok(vec![
        ParameterValue::Str(station.feature_id.clone()),
        ParameterValue::Str(station.station_id.clone()),
        ParameterValue::Str(station.name.clone()),
        ParameterValue::Str(station.country.clone()),
        ParameterValue::Str(station.owner.clone()),
        ParameterValue::Str(station.station_type.clone()),
        ParameterValue::Str(station.status.clone()),
        ParameterValue::Floating64(station.longitude),
        ParameterValue::Floating64(station.latitude),
        ParameterValue::ArrayStr(station.parameter_ids.iter().cloned().map(Some).collect()),
        option_str(&station.region_id),
        option_str(&station.wmo_country_code),
        option_str(&station.wmo_station_id),
        option_f64(station.station_height),
        option_f64(station.barometer_height),
        ParameterValue::Str(station.created.clone()),
        option_str(&station.updated),
        ParameterValue::Str(station.operation_from.clone()),
        option_str(&station.operation_to),
        ParameterValue::Str(station.valid_from.clone()),
        option_str(&station.valid_to),
        ParameterValue::Jsonb(source_json),
    ])
}

fn option_str(value: &Option<String>) -> ParameterValue {
    match value {
        Some(value) => ParameterValue::Str(value.clone()),
        None => ParameterValue::DbNull,
    }
}

fn option_f64(value: Option<f64>) -> ParameterValue {
    match value {
        Some(value) => ParameterValue::Floating64(value),
        None => ParameterValue::DbNull,
    }
}

async fn execute_simple(connection: &Connection, statement: &str) -> Result<u64> {
    connection
        .execute(statement, Vec::<ParameterValue>::new())
        .await
        .with_context(|| format!("execute SQL: {statement}"))
}

fn parse_retry_after_ms(value: &str) -> Option<u64> {
    value.parse::<u64>().ok().map(|seconds| seconds * 1_000)
}

#[cfg(test)]
mod tests {
    use super::parse_retry_after_ms;

    #[test]
    fn parses_retry_after_seconds() {
        assert_eq!(parse_retry_after_ms("30"), Some(30_000));
    }

    #[test]
    fn ignores_non_numeric_retry_after_values() {
        assert_eq!(parse_retry_after_ms("Sat, 18 Oct 2026 01:02:03 GMT"), None);
    }
}
