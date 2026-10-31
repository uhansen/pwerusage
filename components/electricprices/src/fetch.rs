use std::collections::BTreeMap;

use chrono::{DateTime, Duration, Utc};

use crate::{
    FetchError, FetchSummary, Position, PriceKind, PricePoint, Provider, http,
    model::{self, PriceRange, PriceResponse, ZoneResponse},
    pwrusage::electric_prices::storage,
};

pub(crate) async fn run(
    provider: Provider,
    position: Position,
    history_from: Option<String>,
    history_to: Option<String>,
) -> Result<FetchSummary, FetchError> {
    model::validate_position(&position)?;
    let rate = match std::env::var("EUR_DKK_RATE") {
        Ok(value) => value.parse::<f64>().map_err(|_| {
            FetchError::Config("EUR_DKK_RATE must be a finite positive number".into())
        })?,
        Err(std::env::VarError::NotPresent) => 7.46,
        Err(_) => {
            return Err(FetchError::Config(
                "EUR_DKK_RATE is not valid Unicode".into(),
            ));
        }
    };
    model::validate_provider(&provider, rate)?;
    let token = std::env::var("ELECTRICITYMAPS_API_TOKEN")
        .map_err(|_| FetchError::Config("ELECTRICITYMAPS_API_TOKEN is required".into()))?;
    if token.trim().is_empty() || token.bytes().any(|b| b.is_ascii_control()) {
        return Err(FetchError::Config("API token is empty or invalid".into()));
    }
    let zone = match position {
        Position::Zone(zone) => zone,
        Position::Coordinates(c) => {
            let response: ZoneResponse = http::json(
                &format!("/v3/zone?lat={}&lon={}", c.latitude, c.longitude),
                &token,
            )
            .await?;
            model::validate_zone(&response.zone_key)?;
            response.zone_key
        }
    };
    let now = Utc::now();
    let today_date = now
        .with_timezone(&chrono_tz::Europe::Copenhagen)
        .date_naive();
    let today = model::midnight(today_date)?;
    let upcoming_end = model::midnight(today_date + Duration::days(2))?;
    let last = storage::last_history(zone.clone(), provider.name.clone())
        .await
        .map_err(FetchError::Storage)?;
    let (start, end) = model::history_window(
        history_from.as_deref(),
        history_to.as_deref(),
        last.as_deref(),
        today,
    )?;
    let mut points = BTreeMap::new();
    let mut unavailable = 0_u32;
    for (start, end) in model::chunks(start, end) {
        let response: PriceRange = http::json(
            &range_path("/v3/price-day-ahead/past-range", &zone, start, end),
            &token,
        )
        .await?;
        add_range(
            response,
            &provider,
            &zone,
            start,
            end,
            None,
            rate,
            &mut points,
            &mut unavailable,
        )?;
    }
    let latest: PriceResponse = http::json(
        &format!("/v3/price-day-ahead/latest?zone={zone}&temporalGranularity=hourly"),
        &token,
    )
    .await?;
    if latest.zone != zone {
        return Err(FetchError::InvalidData("latest price zone mismatch".into()));
    }
    let latest_time = model::timestamp(&latest.datetime)?;
    if latest_time > now {
        return Err(FetchError::InvalidData(
            "latest price is in the future".into(),
        ));
    }
    if let Some(value) = latest.value {
        let kind = if latest_time < today {
            PriceKind::History
        } else {
            PriceKind::Latest
        };
        let point = model::price_point(
            &provider,
            &zone,
            &latest.datetime,
            value,
            &latest.unit,
            kind,
            rate,
        )?;
        // A current snapshot must not overwrite a history correction from this run.
        points.entry(latest_time).or_insert(point);
    } else {
        unavailable += 1;
    }
    // Upcoming day-ahead values are published/cleared prices, not modelled forecasts.
    let upcoming: PriceRange = http::json(
        &range_path("/v4/price-day-ahead/actual", &zone, today, upcoming_end),
        &token,
    )
    .await?;
    add_range(
        upcoming,
        &provider,
        &zone,
        today,
        upcoming_end,
        Some(now),
        rate,
        &mut points,
        &mut unavailable,
    )?;
    let points: Vec<_> = points.into_values().collect();
    if points.is_empty() {
        return Err(FetchError::InvalidData(
            "API returned no available prices; nothing was persisted".into(),
        ));
    }
    let mut summary = FetchSummary {
        zone,
        history_records: 0,
        latest_records: 0,
        forecast_records: 0,
        unavailable_records: unavailable,
        persisted_records: 0,
    };
    for point in &points {
        match point.kind {
            PriceKind::History => summary.history_records += 1,
            PriceKind::Latest => summary.latest_records += 1,
            PriceKind::Forecast => summary.forecast_records += 1,
        }
    }
    summary.persisted_records = storage::persist(provider, points)
        .await
        .map_err(FetchError::Storage)?;
    Ok(summary)
}

fn range_path(path: &str, zone: &str, start: DateTime<Utc>, end: DateTime<Utc>) -> String {
    format!(
        "{path}?zone={zone}&start={}&end={}&temporalGranularity=hourly",
        model::stamp(start),
        model::stamp(end),
    )
}

#[allow(clippy::too_many_arguments)]
fn add_range(
    response: PriceRange,
    provider: &Provider,
    zone: &str,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    now: Option<DateTime<Utc>>,
    rate: f64,
    points: &mut BTreeMap<DateTime<Utc>, PricePoint>,
    unavailable: &mut u32,
) -> Result<(), FetchError> {
    if response.zone != zone
        || response
            .temporal_granularity
            .as_deref()
            .is_some_and(|g| g != "hourly")
    {
        return Err(FetchError::InvalidData(
            "price range zone or temporal granularity mismatch".into(),
        ));
    }
    let mut seen = std::collections::HashSet::new();
    for row in response.data {
        let datetime = model::timestamp(&row.datetime)?;
        if datetime < start
            || datetime >= end
            || row.zone.as_deref().is_some_and(|z| z != zone)
            || !seen.insert(datetime)
        {
            return Err(FetchError::InvalidData(
                "price row is duplicated, outside the requested range or in another zone".into(),
            ));
        }
        let Some(value) = row.value else {
            *unavailable += 1;
            continue;
        };
        let kind = match now {
            None => PriceKind::History,
            Some(now) if datetime > now => PriceKind::Forecast,
            Some(_) => PriceKind::Latest,
        };
        let point =
            model::price_point(provider, zone, &row.datetime, value, &row.unit, kind, rate)?;
        points.insert(datetime, point);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_ranges_without_turning_unavailable_values_into_zero() {
        let provider = Provider {
            name: "Example".into(),
            markup_per_kwh: 0.0,
            markup_currency: "DKK".into(),
            include_vat: false,
        };
        let payload = json!({
            "zone": "DK-DK1", "temporalGranularity": "hourly",
            "data": [
                {"datetime": "2026-01-01T00:00:00Z", "value": -20.0, "unit": "EUR/MWh"},
                {"datetime": "2026-01-01T01:00:00Z", "value": null, "unit": "EUR/MWh"},
                {"datetime": "2026-01-01T02:00:00Z", "value": 0.0, "unit": "EUR/MWh"}
            ]
        });
        let start = model::timestamp("2026-01-01T00:00:00Z").unwrap();
        let end = start + Duration::days(1);
        let mut points = BTreeMap::new();
        let mut missing = 0;
        add_range(
            serde_json::from_value(payload.clone()).unwrap(),
            &provider,
            "DK-DK1",
            start,
            end,
            Some(start),
            7.46,
            &mut points,
            &mut missing,
        )
        .unwrap();
        assert_eq!(points.len(), 2);
        assert_eq!(missing, 1);
        assert!(matches!(points[&start].kind, PriceKind::Latest));
        assert!(matches!(
            points[&(start + Duration::hours(2))].kind,
            PriceKind::Forecast
        ));
        for invalid in [
            json!({"zone":"DE","data":[]}),
            json!({"zone":"DK-DK1","data":[
                {"datetime":"2026-01-02T00:00:00Z","value":1,"unit":"EUR/MWh"}]}),
            json!({"zone":"DK-DK1","data":[
                {"datetime":"2026-01-01T00:00:00Z","value":1,"unit":"EUR/MWh"},
                {"datetime":"2026-01-01T00:00:00Z","value":2,"unit":"EUR/MWh"}]}),
        ] {
            assert!(
                add_range(
                    serde_json::from_value(invalid).unwrap(),
                    &provider,
                    "DK-DK1",
                    start,
                    end,
                    None,
                    7.46,
                    &mut BTreeMap::new(),
                    &mut 0
                )
                .is_err()
            );
        }
        assert!(serde_json::from_value::<PriceRange>(json!({"zone":"DK-DK1"})).is_err());
    }
}
