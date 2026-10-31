use anyhow::{Context, Result};
use tokio_postgres::{Client, NoTls};

use crate::bindings::pwrusage::electric_prices::types::{PriceKind, PricePoint, Provider};

pub const MIGRATION: &str = include_str!("../../migrations/001_prices.sql");
pub const LOCK_ID: i64 = 8_327_304_551_219_105;

pub async fn connect(url: &str) -> Result<Client> {
    let (client, connection) = tokio_postgres::connect(url, NoTls)
        .await
        .context("connect to PostgreSQL (local/trusted server; no TLS)")?;
    tokio::spawn(async move {
        if let Err(error) = connection.await {
            eprintln!("PostgreSQL connection error: {error}");
        }
    });
    Ok(client)
}

fn kind_name(kind: PriceKind) -> &'static str {
    match kind {
        PriceKind::History => "history",
        PriceKind::Latest => "latest",
        PriceKind::Forecast => "forecast",
    }
}

pub async fn last_history(client: &Client, zone: &str, provider: &str) -> Result<Option<String>> {
    client
        .query_one(
            "SELECT to_char(max(datetime) AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"')
         FROM electricity_prices WHERE zone=$1 AND provider_name=$2 AND kind='history'",
            &[&zone, &provider],
        )
        .await?
        .try_get(0)
        .context("read history watermark")
}

pub async fn persist(
    client: &mut Client,
    provider: &Provider,
    points: &[PricePoint],
) -> Result<u32> {
    anyhow::ensure!(!provider.name.trim().is_empty(), "provider name is empty");
    anyhow::ensure!(
        provider.markup_per_kwh.is_finite(),
        "invalid provider markup"
    );
    let transaction = client
        .transaction()
        .await
        .context("start prices transaction")?;
    let statement = transaction
        .prepare(
            "INSERT INTO electricity_prices
         (zone,datetime,provider_name,kind,raw_value,raw_unit,eur_dkk_rate,spot_dkk_kwh,
          provider_markup_per_kwh,provider_markup_currency,markup_dkk_kwh,include_vat,total_dkk_kwh)
         VALUES ($1,$2::text::timestamptz,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)
         ON CONFLICT (zone,datetime,provider_name) DO UPDATE SET
         kind=EXCLUDED.kind,raw_value=EXCLUDED.raw_value,raw_unit=EXCLUDED.raw_unit,
         eur_dkk_rate=EXCLUDED.eur_dkk_rate,spot_dkk_kwh=EXCLUDED.spot_dkk_kwh,
         provider_markup_per_kwh=EXCLUDED.provider_markup_per_kwh,
         provider_markup_currency=EXCLUDED.provider_markup_currency,
         markup_dkk_kwh=EXCLUDED.markup_dkk_kwh,include_vat=EXCLUDED.include_vat,
         total_dkk_kwh=EXCLUDED.total_dkk_kwh,fetched_at=now()
         WHERE electricity_prices.kind='forecast' OR EXCLUDED.kind='history'
            OR (electricity_prices.kind='latest' AND EXCLUDED.kind='latest')",
        )
        .await?;
    let mut count = 0_u32;
    for point in points {
        anyhow::ensure!(
            [
                point.raw_value,
                point.eur_dkk_rate,
                point.spot_dkk_kwh,
                point.markup_dkk_kwh,
                point.total_dkk_kwh
            ]
            .iter()
            .all(|value| value.is_finite()),
            "non-finite price"
        );
        let updated = transaction
            .execute(
                &statement,
                &[
                    &point.zone,
                    &point.datetime,
                    &provider.name,
                    &kind_name(point.kind),
                    &point.raw_value,
                    &point.raw_unit,
                    &point.eur_dkk_rate,
                    &point.spot_dkk_kwh,
                    &provider.markup_per_kwh,
                    &provider.markup_currency,
                    &point.markup_dkk_kwh,
                    &provider.include_vat,
                    &point.total_dkk_kwh,
                ],
            )
            .await
            .context("upsert price")?;
        count = count
            .checked_add(u32::try_from(updated)?)
            .context("too many persisted prices")?;
    }
    transaction
        .commit()
        .await
        .context("commit prices transaction")?;
    Ok(count)
}
