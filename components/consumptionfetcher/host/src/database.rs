use anyhow::{Context, Result};
use tokio_postgres::{Client, NoTls};

use crate::bindings::pwrusage::consumption_fetcher::types::{
    Direction, Granularity, MeteringPoint, UsagePeriod,
};

pub const MIGRATION: &str = include_str!("../../migrations/001_usage.sql");
// One fetch holds this session lock across reading history and persisting rollups.
pub const LOCK_ID: i64 = 8_327_304_551_219_104;

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

pub fn direction_name(value: Direction) -> &'static str {
    match value {
        Direction::Consumption => "consumption",
        Direction::Production => "production",
        Direction::GridExport => "grid-export",
    }
}

fn parse_direction(value: &str) -> Result<Direction> {
    match value {
        "consumption" => Ok(Direction::Consumption),
        "production" => Ok(Direction::Production),
        "grid-export" => Ok(Direction::GridExport),
        _ => anyhow::bail!("unknown stored direction"),
    }
}

fn granularity_name(value: Granularity) -> &'static str {
    match value {
        Granularity::Day => "day",
        Granularity::Week => "week",
        Granularity::Month => "month",
        Granularity::Year => "year",
    }
}

pub async fn load_days(client: &Client, id: &str) -> Result<Vec<UsagePeriod>> {
    let rows = client
        .query(
            "SELECT direction, period_start::text, period_end::text, milli_kwh, quality, complete
         FROM usage_periods WHERE metering_point_id=$1 AND granularity='day' ORDER BY period_start",
            &[&id],
        )
        .await
        .context("load daily history")?;
    rows.into_iter()
        .map(|row| {
            Ok(UsagePeriod {
                metering_point_id: id.into(),
                direction: parse_direction(row.try_get::<_, &str>(0)?)?,
                granularity: Granularity::Day,
                period_start: row.try_get(1)?,
                period_end: row.try_get(2)?,
                milli_kwh: row.try_get(3)?,
                quality: row.try_get(4)?,
                complete: row.try_get(5)?,
            })
        })
        .collect()
}

pub async fn persist(
    client: &mut Client,
    points: &[MeteringPoint],
    periods: &[UsagePeriod],
) -> Result<u32> {
    let transaction = client
        .transaction()
        .await
        .context("start usage transaction")?;
    let point_statement = transaction.prepare(
        "INSERT INTO metering_points (id,type_of_mp,street_name,building_number,floor,door,postcode,city,direction)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)
         ON CONFLICT (id) DO UPDATE SET
         type_of_mp=EXCLUDED.type_of_mp,street_name=EXCLUDED.street_name,
         building_number=EXCLUDED.building_number,floor=EXCLUDED.floor,door=EXCLUDED.door,
         postcode=EXCLUDED.postcode,city=EXCLUDED.city,direction=EXCLUDED.direction,updated_at=now()"
    ).await?;
    for point in points {
        let a = &point.address;
        transaction
            .execute(
                &point_statement,
                &[
                    &point.id,
                    &point.type_of_mp,
                    &a.street_name,
                    &a.building_number,
                    &a.floor,
                    &a.door,
                    &a.postcode,
                    &a.city,
                    &direction_name(point.direction),
                ],
            )
            .await
            .context("upsert metering point")?;
    }
    let statement = transaction.prepare(
        "INSERT INTO usage_periods (metering_point_id,direction,granularity,period_start,period_end,milli_kwh,quality,complete)
         VALUES ($1,$2,$3,$4::text::date,$5::text::date,$6,$7,$8)
         ON CONFLICT (metering_point_id,direction,granularity,period_start) DO UPDATE SET
         period_end=EXCLUDED.period_end,milli_kwh=EXCLUDED.milli_kwh,quality=EXCLUDED.quality,
         complete=EXCLUDED.complete,fetched_at=now()"
    ).await?;
    for row in periods {
        let point = points
            .iter()
            .find(|point| point.id == row.metering_point_id)
            .context("period refers to an unrequested metering point")?;
        anyhow::ensure!(
            point.direction == row.direction,
            "period direction mismatch"
        );
        transaction
            .execute(
                &statement,
                &[
                    &row.metering_point_id,
                    &direction_name(row.direction),
                    &granularity_name(row.granularity),
                    &row.period_start,
                    &row.period_end,
                    &row.milli_kwh,
                    &row.quality,
                    &row.complete,
                ],
            )
            .await
            .context("upsert usage period")?;
    }
    transaction
        .commit()
        .await
        .context("commit usage transaction")?;
    u32::try_from(periods.len()).context("too many persisted periods")
}
