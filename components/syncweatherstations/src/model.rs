use serde::Deserialize;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use crate::pwrusage::weather_sync::types::StationVersion;

#[derive(Debug, Deserialize)]
pub struct FeatureCollection {
    #[serde(rename = "type")]
    collection_type: String,
    pub features: Vec<Feature>,
    #[serde(rename = "numberReturned")]
    number_returned: u64,
    pub links: Vec<Link>,
}

#[derive(Debug, Deserialize)]
pub struct Link {
    pub href: String,
    pub rel: String,
}

#[derive(Debug, Deserialize)]
pub struct Feature {
    #[serde(rename = "type")]
    feature_type: String,
    id: String,
    geometry: Geometry,
    properties: Properties,
}

#[derive(Debug, Deserialize)]
struct Geometry {
    #[serde(rename = "type")]
    geometry_type: String,
    coordinates: Vec<f64>,
}

#[derive(Debug, Deserialize)]
struct Properties {
    #[serde(rename = "stationId")]
    station_id: String,
    name: String,
    country: String,
    owner: String,
    #[serde(rename = "type")]
    station_type: String,
    status: String,
    #[serde(rename = "parameterId")]
    parameter_ids: Vec<String>,
    #[serde(rename = "regionId")]
    region_id: Option<String>,
    #[serde(rename = "wmoCountryCode", alias = "wmoContryCode")]
    wmo_country_code: Option<String>,
    #[serde(rename = "wmoStationId")]
    wmo_station_id: Option<String>,
    #[serde(rename = "stationHeight")]
    station_height: Option<f64>,
    #[serde(rename = "barometerHeight")]
    barometer_height: Option<f64>,
    created: String,
    updated: Option<String>,
    #[serde(rename = "operationFrom")]
    operation_from: String,
    #[serde(rename = "operationTo")]
    operation_to: Option<String>,
    #[serde(rename = "validFrom")]
    valid_from: String,
    #[serde(rename = "validTo")]
    valid_to: Option<String>,
}

impl FeatureCollection {
    pub fn validate(&self) -> Result<(), String> {
        if self.collection_type != "FeatureCollection" {
            return Err("response type must be FeatureCollection".to_string());
        }
        if self.number_returned != self.features.len() as u64 {
            return Err(format!(
                "numberReturned {} does not match feature count {}",
                self.number_returned,
                self.features.len()
            ));
        }
        Ok(())
    }
}

impl Feature {
    pub fn into_station(self, source_json: String) -> Result<StationVersion, String> {
        if self.feature_type != "Feature" {
            return Err("feature type must be Feature".to_string());
        }
        if self.id.trim().is_empty() {
            return Err("feature id must not be empty".to_string());
        }
        if self.geometry.geometry_type != "Point" || self.geometry.coordinates.len() != 2 {
            return Err(format!("feature {} must have Point coordinates", self.id));
        }

        let longitude = self.geometry.coordinates[0];
        let latitude = self.geometry.coordinates[1];
        if !longitude.is_finite()
            || !latitude.is_finite()
            || !(-180.0..=180.0).contains(&longitude)
            || !(-90.0..=90.0).contains(&latitude)
        {
            return Err(format!("feature {} has invalid coordinates", self.id));
        }

        let p = self.properties;
        for (field, value) in [
            ("stationId", p.station_id.as_str()),
            ("name", p.name.as_str()),
            ("country", p.country.as_str()),
            ("status", p.status.as_str()),
        ] {
            if value.trim().is_empty() {
                return Err(format!("feature {} has empty {field}", self.id));
            }
        }

        validate_timestamp("created", &p.created)?;
        validate_timestamp("operationFrom", &p.operation_from)?;
        validate_timestamp("validFrom", &p.valid_from)?;
        validate_optional_timestamp("updated", p.updated.as_deref())?;
        validate_optional_timestamp("operationTo", p.operation_to.as_deref())?;
        validate_optional_timestamp("validTo", p.valid_to.as_deref())?;
        validate_interval("operation", &p.operation_from, p.operation_to.as_deref())?;
        validate_interval("validity", &p.valid_from, p.valid_to.as_deref())?;

        for (field, value) in [
            ("stationHeight", p.station_height),
            ("barometerHeight", p.barometer_height),
        ] {
            if value.is_some_and(|number| !number.is_finite()) {
                return Err(format!("feature {} has invalid {field}", self.id));
            }
        }

        if p.parameter_ids.iter().any(|id| id.trim().is_empty()) {
            return Err(format!("feature {} has an empty parameterId", self.id));
        }

        Ok(StationVersion {
            feature_id: self.id,
            station_id: p.station_id,
            name: p.name,
            country: p.country,
            owner: p.owner,
            station_type: p.station_type,
            status: p.status,
            longitude,
            latitude,
            parameter_ids: p.parameter_ids,
            region_id: p.region_id,
            wmo_country_code: p.wmo_country_code,
            wmo_station_id: p.wmo_station_id,
            station_height: p.station_height,
            barometer_height: p.barometer_height,
            created: p.created,
            updated: p.updated,
            operation_from: p.operation_from,
            operation_to: p.operation_to,
            valid_from: p.valid_from,
            valid_to: p.valid_to,
            source_json,
        })
    }
}

fn parse_timestamp(field: &str, value: &str) -> Result<OffsetDateTime, String> {
    OffsetDateTime::parse(value, &Rfc3339)
        .map_err(|error| format!("{field} is not RFC3339: {error}"))
}

fn validate_timestamp(field: &str, value: &str) -> Result<(), String> {
    parse_timestamp(field, value).map(|_| ())
}

fn validate_optional_timestamp(field: &str, value: Option<&str>) -> Result<(), String> {
    value.map_or(Ok(()), |value| validate_timestamp(field, value))
}

fn validate_interval(field: &str, from: &str, to: Option<&str>) -> Result<(), String> {
    let from = parse_timestamp(field, from)?;
    if let Some(to) = to {
        let to = parse_timestamp(field, to)?;
        if to < from {
            return Err(format!("{field} end precedes start"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_historical_station_with_empty_owner_and_type() {
        let json = serde_json::json!({
            "type": "Feature",
            "id": "5a2de1d9-8f33-c428-9d28-8bfa722e6ae0",
            "geometry": {"type": "Point", "coordinates": [8.5991, 57.1125]},
            "properties": {
                "owner": "",
                "country": "DNK",
                "operationFrom": "1961-01-01T00:00:00Z",
                "parameterId": ["temp_dry"],
                "created": "2026-09-29T10:36:49Z",
                "barometerHeight": null,
                "validFrom": "1961-01-01T00:00:00Z",
                "type": "",
                "stationHeight": 46.0,
                "regionId": "6",
                "name": "HANSTHOLM FYR",
                "wmoCountryCode": "6080",
                "wmoStationId": "06021",
                "operationTo": null,
                "updated": null,
                "stationId": "06021",
                "validTo": "1981-09-21T00:00:00Z",
                "status": "Active"
            }
        });
        let feature: Feature = serde_json::from_value(json.clone()).unwrap();
        let station = feature.into_station(json.to_string()).unwrap();

        assert_eq!(station.station_id, "06021");
        assert_eq!(station.owner, "");
        assert_eq!(station.station_type, "");
        assert_eq!(station.longitude, 8.5991);
        assert_eq!(station.latitude, 57.1125);
    }

    #[test]
    fn rejects_invalid_coordinates_and_intervals() {
        assert!(
            validate_interval(
                "validity",
                "2026-01-02T00:00:00Z",
                Some("2026-01-01T00:00:00Z")
            )
            .is_err()
        );
        assert!(parse_timestamp("created", "not-a-timestamp").is_err());
    }
}
