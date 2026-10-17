use std::collections::{HashMap, HashSet};

use serde_json::Value;
use url::Url;

use crate::{
    model::FeatureCollection,
    pwrusage::weather_sync::{
        storage, transport,
        types::{StationVersion, SyncError, SyncSummary},
    },
};

const FIRST_PAGE: &str =
    "https://opendataapi.dmi.dk/v2/metObs/collections/station/items?limit=1000";
const ALLOWED_ORIGIN: &str = "https://opendataapi.dmi.dk";
const ALLOWED_PATH: &str = "/v2/metObs/collections/station/items";
const MAX_PAGES: u32 = 1_000;
const MAX_STATIONS: usize = 500_000;
const MAX_BODY_BYTES: usize = 32 * 1024 * 1024;
const MAX_RETRIES: u32 = 3;
const MAX_RETRY_AFTER_MS: u64 = 30_000;

pub async fn run() -> Result<SyncSummary, SyncError> {
    let mut next = Some(FIRST_PAGE.to_string());
    let mut visited = HashSet::new();
    let mut stations = HashMap::<String, (StationVersion, String)>::new();
    let mut pages = 0_u32;
    let mut received = 0_u64;

    while let Some(url) = next {
        validate_url(&url)?;
        if !visited.insert(url.clone()) {
            return Err(SyncError::InvalidData(format!(
                "pagination loop detected at {url}"
            )));
        }
        if pages >= MAX_PAGES {
            return Err(SyncError::LimitExceeded(format!(
                "page count exceeds {MAX_PAGES}"
            )));
        }

        let response = get_with_retry(&url).await?;
        if response.body.len() > MAX_BODY_BYTES {
            return Err(SyncError::LimitExceeded(format!(
                "response body exceeds {MAX_BODY_BYTES} bytes"
            )));
        }

        let raw: Value = serde_json::from_str(&response.body)
            .map_err(|error| SyncError::InvalidData(format!("invalid JSON: {error}")))?;
        let collection: FeatureCollection = serde_json::from_value(raw.clone())
            .map_err(|error| SyncError::InvalidData(format!("invalid DMI response: {error}")))?;
        collection.validate().map_err(SyncError::InvalidData)?;

        pages += 1;
        received += collection.features.len() as u64;

        let raw_features = raw
            .get("features")
            .and_then(Value::as_array)
            .ok_or_else(|| SyncError::InvalidData("features must be an array".to_string()))?;
        if raw_features.len() != collection.features.len() {
            return Err(SyncError::InvalidData(
                "decoded feature count mismatch".to_string(),
            ));
        }

        for (feature, raw_feature) in collection.features.into_iter().zip(raw_features) {
            let source_json = serde_json::to_string(raw_feature).map_err(|error| {
                SyncError::InvalidData(format!("cannot preserve source feature: {error}"))
            })?;
            let station = feature
                .into_station(source_json.clone())
                .map_err(SyncError::InvalidData)?;

            match stations.get(&station.feature_id) {
                Some((_, existing_json)) if existing_json != &source_json => {
                    return Err(SyncError::InvalidData(format!(
                        "conflicting duplicate feature {}",
                        station.feature_id
                    )));
                }
                Some(_) => {}
                None => {
                    stations.insert(station.feature_id.clone(), (station, source_json));
                }
            }

            if stations.len() > MAX_STATIONS {
                return Err(SyncError::LimitExceeded(format!(
                    "station count exceeds {MAX_STATIONS}"
                )));
            }
        }

        next = next_link(&collection.links)?;
    }

    if stations.is_empty() {
        return Err(SyncError::InvalidData(
            "DMI returned an empty station catalog".to_string(),
        ));
    }

    let mut stations: Vec<_> = stations.into_values().map(|(station, _)| station).collect();
    stations.sort_by(|left, right| left.feature_id.cmp(&right.feature_id));
    let unique = stations.len() as u64;
    let persisted = storage::persist(stations)
        .await
        .map_err(SyncError::Storage)?;

    Ok(SyncSummary {
        pages,
        received,
        unique,
        inserted: persisted.inserted,
        changed: persisted.changed,
        unchanged: persisted.unchanged,
    })
}

async fn get_with_retry(url: &str) -> Result<transport::Response, SyncError> {
    let mut attempts = 0_u32;
    loop {
        let response = match transport::get(url.to_string()).await {
            Ok(response) => response,
            Err(_) if attempts < MAX_RETRIES => {
                sleep_before_retry(attempts, None).await;
                attempts += 1;
                continue;
            }
            Err(error) => {
                return Err(SyncError::Transport(format!(
                    "{error} after {MAX_RETRIES} retries"
                )));
            }
        };
        if (200..300).contains(&response.status) {
            return Ok(response);
        }
        if response.status != 429 && !(500..600).contains(&response.status) {
            return Err(SyncError::Upstream(format!(
                "DMI returned HTTP {}",
                response.status
            )));
        }
        if attempts >= MAX_RETRIES {
            return Err(SyncError::Upstream(format!(
                "DMI returned HTTP {} after {} retries",
                response.status, MAX_RETRIES
            )));
        }
        sleep_before_retry(attempts, response.retry_after_ms).await;
        attempts += 1;
    }
}

async fn sleep_before_retry(attempts: u32, retry_after_ms: Option<u64>) {
    let exponential = 1_000_u64 << attempts;
    let delay = retry_after_ms
        .unwrap_or(exponential)
        .min(MAX_RETRY_AFTER_MS);
    transport::sleep_ms(delay).await;
}

fn next_link(links: &[crate::model::Link]) -> Result<Option<String>, SyncError> {
    let next: Vec<_> = links.iter().filter(|link| link.rel == "next").collect();
    match next.as_slice() {
        [] => Ok(None),
        [link] => {
            validate_url(&link.href)?;
            Ok(Some(link.href.clone()))
        }
        _ => Err(SyncError::InvalidData(
            "response contains multiple next links".to_string(),
        )),
    }
}

fn validate_url(value: &str) -> Result<(), SyncError> {
    let url = Url::parse(value)
        .map_err(|error| SyncError::InvalidData(format!("invalid page URL: {error}")))?;
    if url.origin().ascii_serialization() != ALLOWED_ORIGIN
        || url.path() != ALLOWED_PATH
        || url.username() != ""
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(SyncError::InvalidData(format!(
            "untrusted page URL: {value}"
        )));
    }

    let mut limit_seen = false;
    let mut offset_seen = false;
    for (name, value) in url.query_pairs() {
        match name.as_ref() {
            "limit" if !limit_seen && value == "1000" => limit_seen = true,
            "offset" if !offset_seen => {
                let offset = value.parse::<u64>().map_err(|_| {
                    SyncError::InvalidData(format!("invalid pagination offset in {value}"))
                })?;
                if offset > 500_000 {
                    return Err(SyncError::LimitExceeded(format!(
                        "pagination offset exceeds 500000: {offset}"
                    )));
                }
                offset_seen = true;
            }
            _ => {
                return Err(SyncError::InvalidData(format!(
                    "unexpected or duplicate page query parameter: {name}"
                )));
            }
        }
    }
    if !limit_seen {
        return Err(SyncError::InvalidData(
            "page URL must specify limit=1000".to_string(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_expected_station_page_urls() {
        assert!(validate_url(FIRST_PAGE).is_ok());
        assert!(
            validate_url(
                "https://opendataapi.dmi.dk/v2/metObs/collections/station/items?limit=1000&offset=1000"
            )
            .is_ok()
        );
    }

    #[test]
    fn rejects_untrusted_or_unbounded_page_urls() {
        for url in [
            "https://example.com/v2/metObs/collections/station/items?limit=1000",
            "https://opendataapi.dmi.dk/v2/metObs/collections/observation/items?limit=1000",
            "https://opendataapi.dmi.dk/v2/metObs/collections/station/items?limit=9999",
            "https://opendataapi.dmi.dk/v2/metObs/collections/station/items?limit=1000&offset=500001",
            "https://opendataapi.dmi.dk/v2/metObs/collections/station/items?limit=1000&status=Active",
        ] {
            assert!(validate_url(url).is_err(), "{url} should be rejected");
        }
    }
}
