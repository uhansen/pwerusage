use std::collections::BTreeMap;

use chrono::{Datelike, Duration, NaiveDate};

use crate::pwrusage::consumption_fetcher::storage;
use crate::{Address, FetchError, FetchSummary, Granularity, UsagePeriod, http, model};

pub(crate) async fn run(
    address: Address,
    date_from: Option<String>,
    date_to: Option<String>,
) -> Result<FetchSummary, FetchError> {
    let today = model::today();
    let earliest = NaiveDate::from_ymd_opt(today.year() - 5, 1, 1).unwrap();
    let from = date_from.as_deref().map(model::date).transpose()?;
    let to = date_to
        .as_deref()
        .map(model::date)
        .transpose()?
        .unwrap_or(today);
    if to > today || to <= earliest || from.is_some_and(|from| from < earliest || from >= to) {
        return Err(FetchError::InvalidData("range must be within the previous five years plus current year; date-to is exclusive and cannot exceed today".into()));
    }
    let token = std::env::var("ELOVERBLIK_REFRESH_TOKEN")
        .map_err(|_| FetchError::Config("ELOVERBLIK_REFRESH_TOKEN is required".into()))?;
    if token.trim().is_empty()
        || token
            .bytes()
            .any(|b| b.is_ascii_whitespace() || b.is_ascii_control())
    {
        return Err(FetchError::Config(
            "refresh token must be nonempty and contain no whitespace".into(),
        ));
    }
    let access: model::Envelope<String> = http::json("/token", &token, None).await?;
    if access.result.is_empty()
        || access
            .result
            .bytes()
            .any(|b| b.is_ascii_whitespace() || b.is_ascii_control())
    {
        return Err(FetchError::InvalidData(
            "invalid access token returned by API".into(),
        ));
    }
    let response: model::Envelope<Vec<model::ApiPoint>> = http::json(
        "/meteringpoints/meteringpoints?includeAll=true",
        &access.result,
        None,
    )
    .await?;
    let selected = model::select(&address, response.result)?;
    let mut periods = Vec::new();
    let mut points = Vec::new();
    let mut daily_records = 0_u32;
    let mut unavailable_periods = 0_u32;
    for selected in selected {
        let point = selected.point;
        let stored = storage::load_days(point.id.clone())
            .await
            .map_err(FetchError::Storage)?;
        let mut days: BTreeMap<NaiveDate, UsagePeriod> = BTreeMap::new();
        for row in stored {
            if row.metering_point_id != point.id
                || row.direction != point.direction
                || row.granularity != Granularity::Day
            {
                return Err(FetchError::Storage(
                    "stored daily row identity mismatch".into(),
                ));
            }
            let day = model::date(&row.period_start)?;
            if days.insert(day, row).is_some() {
                return Err(FetchError::Storage("duplicate stored daily rows".into()));
            }
        }
        let incremental = days
            .last_key_value()
            .map(|(day, _)| *day - Duration::days(7));
        let mut start = from.or(incremental).unwrap_or(earliest).max(earliest);
        if let Some(available_from) = selected.available_from {
            start = start.max(available_from);
        }
        while start < to {
            let end = (start + Duration::days(730)).min(to);
            let body =
                serde_json::json!({"meteringPoints":{"meteringPoint":[point.id]}}).to_string();
            let path = format!("/meterdata/gettimeseries/{start}/{end}/Day");
            let response: model::Envelope<Vec<model::SeriesResult>> =
                http::json(&path, &access.result, Some(&body)).await?;
            let (received, unavailable) = model::parse_days(&point, response.result, start, end)?;
            daily_records += received.len() as u32;
            unavailable_periods += unavailable;
            for row in received {
                days.insert(model::date(&row.period_start)?, row);
            }
            start = end;
        }
        periods.extend(model::aggregate(days.into_values().collect())?);
        points.push(point);
    }
    let metering_points = points.len() as u32;
    let persisted_records = storage::persist(points, periods)
        .await
        .map_err(FetchError::Storage)?;
    Ok(FetchSummary {
        metering_points,
        daily_records,
        persisted_records,
        unavailable_periods,
    })
}
