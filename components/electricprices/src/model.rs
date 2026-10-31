use chrono::{DateTime, Duration, NaiveDate, SecondsFormat, TimeZone, Utc};
use chrono_tz::Europe::Copenhagen;
use serde::Deserialize;

use crate::{FetchError, Position, PriceKind, PricePoint, Provider};

#[derive(Deserialize)]
pub(crate) struct ZoneResponse {
    #[serde(rename = "zoneKey")]
    pub zone_key: String,
}

#[derive(Deserialize)]
pub(crate) struct PriceResponse {
    pub zone: String,
    pub datetime: String,
    pub value: Option<f64>,
    pub unit: String,
}

#[derive(Deserialize)]
pub(crate) struct PriceRange {
    pub zone: String,
    pub data: Vec<PriceRow>,
    #[serde(rename = "temporalGranularity")]
    pub temporal_granularity: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct PriceRow {
    pub datetime: String,
    pub value: Option<f64>,
    pub unit: String,
    pub zone: Option<String>,
}

pub fn validate_position(position: &Position) -> Result<(), FetchError> {
    match position {
        Position::Coordinates(c)
            if c.latitude.is_finite()
                && c.longitude.is_finite()
                && (-90.0..=90.0).contains(&c.latitude)
                && (-180.0..=180.0).contains(&c.longitude) =>
        {
            Ok(())
        }
        Position::Coordinates(_) => Err(FetchError::InvalidPosition(
            "latitude must be -90..90 and longitude -180..180 degrees, both finite".into(),
        )),
        Position::Zone(zone) => validate_zone(zone),
    }
}

pub fn validate_zone(zone: &str) -> Result<(), FetchError> {
    if zone.is_empty()
        || zone.len() > 80
        || !zone
            .bytes()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == b'-')
    {
        return Err(FetchError::InvalidPosition(
            "zone must be an Electricity Maps key such as DK-DK1 (uppercase letters, digits, hyphens)"
                .into(),
        ));
    }
    Ok(())
}

pub fn validate_provider(provider: &Provider, rate: f64) -> Result<(), FetchError> {
    if provider.name.trim().is_empty() || provider.name.len() > 200 {
        return Err(FetchError::Config(
            "provider name must contain 1..200 bytes".into(),
        ));
    }
    if !rate.is_finite() || rate <= 0.0 || !provider.markup_per_kwh.is_finite() {
        return Err(FetchError::Config(
            "EUR_DKK_RATE must be finite and positive; markup must be finite".into(),
        ));
    }
    currency_factor(&provider.markup_currency, rate)?;
    Ok(())
}

fn currency_factor(currency: &str, rate: f64) -> Result<f64, FetchError> {
    match currency {
        "DKK" => Ok(1.0),
        "EUR" => Ok(rate),
        _ => Err(FetchError::InvalidData(
            "only DKK and EUR currencies are supported; no exchange rate is available for others"
                .into(),
        )),
    }
}

pub fn price_point(
    provider: &Provider,
    zone: &str,
    datetime: &str,
    value: f64,
    unit: &str,
    kind: PriceKind,
    rate: f64,
) -> Result<PricePoint, FetchError> {
    validate_provider(provider, rate)?;
    validate_zone(zone)?;
    let datetime = timestamp(datetime)?;
    if !value.is_finite() {
        return Err(FetchError::InvalidData("price must be finite".into()));
    }
    let (currency, energy) = unit
        .split_once('/')
        .ok_or_else(|| FetchError::InvalidData("expected a price unit such as EUR/MWh".into()))?;
    let divisor = match energy {
        "MWh" => 1000.0,
        "kWh" => 1.0,
        _ => {
            return Err(FetchError::InvalidData(
                "unsupported price energy unit".into(),
            ));
        }
    };
    let spot = value / divisor * currency_factor(currency, rate)?;
    let markup = provider.markup_per_kwh * currency_factor(&provider.markup_currency, rate)?;
    let total = (spot + markup) * if provider.include_vat { 1.25 } else { 1.0 };
    if !spot.is_finite() || !markup.is_finite() || !total.is_finite() {
        return Err(FetchError::InvalidData("price calculation overflow".into()));
    }
    Ok(PricePoint {
        zone: zone.into(),
        datetime: stamp(datetime),
        kind,
        raw_value: value,
        raw_unit: unit.into(),
        eur_dkk_rate: rate,
        spot_dkk_kwh: spot,
        markup_dkk_kwh: markup,
        total_dkk_kwh: total,
    })
}

pub(crate) fn timestamp(value: &str) -> Result<DateTime<Utc>, FetchError> {
    DateTime::parse_from_rfc3339(value)
        .map(|d| d.with_timezone(&Utc))
        .map_err(|_| FetchError::InvalidData("expected an RFC3339 timestamp with timezone".into()))
}

pub(crate) fn stamp(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(SecondsFormat::Secs, true)
}

pub(crate) fn midnight(value: NaiveDate) -> Result<DateTime<Utc>, FetchError> {
    Copenhagen
        .from_local_datetime(&value.and_hms_opt(0, 0, 0).expect("valid midnight"))
        .single()
        .map(|d| d.with_timezone(&Utc))
        .ok_or_else(|| FetchError::Config("invalid Copenhagen midnight".into()))
}

fn date(value: &str) -> Result<DateTime<Utc>, FetchError> {
    let parsed = NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .map_err(|_| FetchError::Config("history dates must be YYYY-MM-DD".into()))?;
    if parsed.format("%Y-%m-%d").to_string() != value {
        return Err(FetchError::Config(
            "history dates must be YYYY-MM-DD".into(),
        ));
    }
    midnight(parsed)
}

pub(crate) fn history_window(
    from: Option<&str>,
    to: Option<&str>,
    last: Option<&str>,
    today: DateTime<Utc>,
) -> Result<(DateTime<Utc>, DateTime<Utc>), FetchError> {
    let end = to.map(date).transpose()?.unwrap_or(today);
    if end > today {
        return Err(FetchError::Config(
            "history-to is exclusive and cannot be after today".into(),
        ));
    }
    let start = if let Some(from) = from {
        date(from)?
    } else if let Some(last) = last {
        (timestamp(last)? - Duration::days(1)).min(end - Duration::days(1))
    } else {
        midnight(end.with_timezone(&Copenhagen).date_naive() - Duration::days(30))?
    };
    if start >= end {
        return Err(FetchError::Config(
            "history-from must precede the exclusive history-to".into(),
        ));
    }
    Ok((start, end))
}

pub(crate) fn chunks(
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Vec<(DateTime<Utc>, DateTime<Utc>)> {
    let mut result = Vec::new();
    let mut cursor = start;
    // Stay below the v3 past-range API's ten-day limit.
    while cursor < end {
        let next = (cursor + Duration::days(5)).min(end);
        result.push((cursor, next));
        cursor = next;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Coordinates;

    fn provider(vat: bool) -> Provider {
        Provider {
            name: "Example supplier".into(),
            markup_per_kwh: 0.1,
            markup_currency: "DKK".into(),
            include_vat: vat,
        }
    }

    #[test]
    fn converts_negative_prices_and_applies_markup_before_optional_vat() {
        let price = price_point(
            &provider(true),
            "DK-DK1",
            "2026-10-25T02:00:00+02:00",
            -50.0,
            "EUR/MWh",
            PriceKind::History,
            7.46,
        )
        .unwrap();
        assert_eq!(price.datetime, "2026-10-25T00:00:00Z");
        assert!((price.spot_dkk_kwh + 0.373).abs() < 1e-12);
        assert!((price.total_dkk_kwh + 0.34125).abs() < 1e-12);
        let mut supplier = provider(false);
        supplier.markup_currency = "EUR".into();
        let price = price_point(
            &supplier,
            "DK-DK2",
            "2026-01-01T00:00:00Z",
            0.5,
            "DKK/kWh",
            PriceKind::Latest,
            7.46,
        )
        .unwrap();
        assert!((price.total_dkk_kwh - 1.246).abs() < 1e-12);
    }

    #[test]
    fn rejects_unsupported_units_invalid_numbers_and_positions() {
        for (value, unit, rate) in [
            (10.0, "USD/MWh", 7.46),
            (10.0, "EUR/Wh", 7.46),
            (f64::INFINITY, "EUR/MWh", 7.46),
            (10.0, "EUR/MWh", 0.0),
            (f64::MAX, "EUR/kWh", f64::MAX),
        ] {
            assert!(
                price_point(
                    &provider(false),
                    "DK-DK1",
                    "2026-01-01T00:00:00Z",
                    value,
                    unit,
                    PriceKind::Latest,
                    rate
                )
                .is_err()
            );
        }
        for zone in ["", "DK-DK1&zone=DE", "dk-dk1", "../zones"] {
            assert!(validate_position(&Position::Zone(zone.into())).is_err());
        }
        for (latitude, longitude) in [(91.0, 0.0), (0.0, 181.0), (f64::NAN, 0.0)] {
            assert!(
                validate_position(&Position::Coordinates(Coordinates {
                    latitude,
                    longitude,
                }))
                .is_err()
            );
        }
    }

    #[test]
    fn history_is_exclusive_chunked_and_handles_dst_and_explicit_backfills() {
        let today = midnight(NaiveDate::from_ymd_opt(2026, 10, 26).unwrap()).unwrap();
        let (start, end) = history_window(Some("2026-10-25"), None, None, today).unwrap();
        assert_eq!((end - start).num_hours(), 25);
        let (start, end) =
            history_window(Some("2026-03-29"), Some("2026-03-30"), None, today).unwrap();
        assert_eq!((end - start).num_hours(), 23);
        let (start, end) = history_window(
            Some("2026-01-01"),
            None,
            Some("2026-10-20T00:00:00Z"),
            today,
        )
        .unwrap();
        assert_eq!(stamp(start), "2025-12-31T23:00:00Z");
        let ranges = chunks(start, end);
        assert_eq!(ranges.first().unwrap().0, start);
        assert_eq!(ranges.last().unwrap().1, end);
        assert!(ranges.iter().all(|(a, b)| *b - *a <= Duration::days(5)));
        assert!(ranges.windows(2).all(|w| w[0].1 == w[1].0));
        assert!(history_window(Some("2026-10-27"), None, None, today).is_err());
        assert!(history_window(None, Some("2026-10-27"), None, today).is_err());
        assert!(history_window(None, None, Some("broken"), today).is_err());
    }
}
