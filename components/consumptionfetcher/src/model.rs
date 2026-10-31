use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Datelike, Duration, NaiveDate, TimeZone};
use chrono_tz::Europe::Copenhagen;
use serde::Deserialize;

use crate::{Address, Direction, FetchError, Granularity, MeteringPoint, UsagePeriod};

fn invalid(message: impl Into<String>) -> FetchError {
    FetchError::InvalidData(message.into())
}

pub fn date(value: &str) -> Result<NaiveDate, FetchError> {
    NaiveDate::parse_from_str(value, "%Y-%m-%d").map_err(|_| invalid("expected YYYY-MM-DD"))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiPoint {
    pub metering_point_id: String,
    #[serde(rename = "typeOfMP")]
    pub type_of_mp: String,
    pub street_name: Option<String>,
    pub building_number: Option<String>,
    pub floor_id: Option<String>,
    pub room_id: Option<String>,
    pub postcode: Option<String>,
    pub city_name: Option<String>,
    pub has_relation: bool,
    #[serde(default)]
    pub is_moved_out: bool,
    pub consumer_start_date: Option<String>,
    pub child_metering_points: Option<Vec<ChildPoint>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChildPoint {
    pub metering_point_id: String,
    #[serde(rename = "typeOfMP")]
    pub type_of_mp: String,
}

pub struct SelectedPoint {
    pub point: MeteringPoint,
    pub available_from: Option<NaiveDate>,
}

fn normalized(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn matches_optional(wanted: &Option<String>, actual: &Option<String>) -> bool {
    wanted
        .as_ref()
        .is_none_or(|wanted| normalized(wanted) == normalized(actual.as_deref().unwrap_or("")))
}

fn direction(code: &str) -> Option<Direction> {
    match code {
        "E17" | "D07" | "D12" => Some(Direction::Consumption),
        "E18" | "D01" | "D05" | "D09" | "D18" => Some(Direction::Production),
        "D04" | "D06" | "D11" => Some(Direction::GridExport),
        _ => None,
    }
}

pub fn select(address: &Address, points: Vec<ApiPoint>) -> Result<Vec<SelectedPoint>, FetchError> {
    if [
        &address.street_name,
        &address.building_number,
        &address.postcode,
    ]
    .iter()
    .any(|v| v.trim().is_empty())
    {
        return Err(invalid("street, building number and postcode are required"));
    }
    let matches: Vec<_> = points
        .into_iter()
        .filter(|p| {
            !p.is_moved_out
                && normalized(p.street_name.as_deref().unwrap_or(""))
                    == normalized(&address.street_name)
                && normalized(p.building_number.as_deref().unwrap_or(""))
                    == normalized(&address.building_number)
                && normalized(p.postcode.as_deref().unwrap_or("")) == normalized(&address.postcode)
                && matches_optional(&address.floor, &p.floor_id)
                && matches_optional(&address.door, &p.room_id)
                && matches_optional(&address.city, &p.city_name)
        })
        .collect();
    if matches.is_empty() {
        return Err(FetchError::AddressNotFound(
            "no active metering point matches this address".into(),
        ));
    }
    let units: BTreeSet<_> = matches
        .iter()
        .map(|p| {
            (
                normalized(p.floor_id.as_deref().unwrap_or("")),
                normalized(p.room_id.as_deref().unwrap_or("")),
            )
        })
        .collect();
    if units.len() > 1 {
        return Err(FetchError::AmbiguousAddress(
            "specify floor and door to identify one household".into(),
        ));
    }
    if matches.iter().any(|p| !p.has_relation) {
        return Err(FetchError::NoRelation(
            "link the household's metering points in the Eloverblik portal".into(),
        ));
    }
    let mut selected = BTreeMap::new();
    for parent in matches {
        let actual_address = Address {
            street_name: parent.street_name.unwrap_or_default(),
            building_number: parent.building_number.unwrap_or_default(),
            postcode: parent.postcode.unwrap_or_default(),
            floor: parent.floor_id,
            door: parent.room_id,
            city: parent.city_name,
        };
        let available_from = parent
            .consumer_start_date
            .as_deref()
            .map(|value| date(value.get(..10).unwrap_or(value)))
            .transpose()?;
        let mut candidates = vec![(parent.metering_point_id, parent.type_of_mp)];
        candidates.extend(
            parent
                .child_metering_points
                .unwrap_or_default()
                .into_iter()
                .map(|child| (child.metering_point_id, child.type_of_mp)),
        );
        for (id, code) in candidates {
            let Some(direction) = direction(&code) else {
                continue;
            };
            if id.len() != 18 || !id.bytes().all(|b| b.is_ascii_digit()) {
                return Err(invalid("metering point ID must contain 18 digits"));
            }
            let candidate = SelectedPoint {
                point: MeteringPoint {
                    id: id.clone(),
                    type_of_mp: code,
                    address: actual_address.clone(),
                    direction,
                },
                available_from,
            };
            if let Some(old) = selected.insert(id, candidate) {
                let new = selected.get(&old.point.id).unwrap();
                if old.point.type_of_mp != new.point.type_of_mp {
                    return Err(invalid("conflicting metering point types"));
                }
            }
        }
    }
    if selected.is_empty() {
        return Err(invalid(
            "no supported consumption or production metering points",
        ));
    }
    Ok(selected.into_values().collect())
}

#[derive(Deserialize)]
pub struct Envelope<T> {
    pub result: T,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SeriesResult {
    pub success: bool,
    pub error_code: i32,
    pub id: String,
    #[serde(rename = "MyEnergyData_MarketDocument")]
    pub document: Option<Document>,
}

#[derive(Deserialize)]
pub struct Document {
    #[serde(rename = "TimeSeries")]
    pub series: Option<Vec<Series>>,
}

#[derive(Deserialize)]
pub struct Series {
    #[serde(rename = "measurement_Unit.name")]
    pub unit: String,
    #[serde(rename = "MarketEvaluationPoint")]
    pub evaluation_point: EvaluationPoint,
    #[serde(rename = "Period")]
    pub periods: Vec<Period>,
}

#[derive(Deserialize)]
pub struct EvaluationPoint {
    #[serde(rename = "mRID")]
    pub id: EvaluationId,
}

#[derive(Deserialize)]
pub struct EvaluationId {
    pub name: String,
}

#[derive(Deserialize)]
pub struct Period {
    pub resolution: String,
    #[serde(rename = "timeInterval")]
    pub interval: Interval,
    #[serde(rename = "Point")]
    pub points: Vec<Point>,
}

#[derive(Deserialize)]
pub struct Interval {
    pub start: String,
    pub end: String,
}

#[derive(Deserialize)]
pub struct Point {
    pub position: String,
    #[serde(rename = "out_Quantity.quantity")]
    pub quantity: Option<String>,
    #[serde(rename = "out_Quantity.quality")]
    pub quality: String,
}

pub fn milli_kwh(value: &str) -> Result<i64, FetchError> {
    let (whole, fraction) = value.split_once('.').unwrap_or((value, ""));
    if whole.is_empty()
        || !whole.bytes().all(|c| c.is_ascii_digit())
        || fraction.len() > 3
        || !fraction.bytes().all(|c| c.is_ascii_digit())
    {
        return Err(invalid(
            "quantity must be a nonnegative decimal with at most 3 places",
        ));
    }
    let whole: i64 = whole.parse().map_err(|_| invalid("quantity overflow"))?;
    let fraction: i64 = format!("{fraction:0<3}")
        .parse()
        .map_err(|_| invalid("invalid fraction"))?;
    whole
        .checked_mul(1000)
        .and_then(|w| w.checked_add(fraction))
        .ok_or_else(|| invalid("quantity overflow"))
}

pub fn parse_days(
    point: &MeteringPoint,
    results: Vec<SeriesResult>,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<(Vec<UsagePeriod>, u32), FetchError> {
    if results.is_empty() {
        return Err(invalid("API omitted the requested metering point"));
    }
    let mut days: BTreeMap<NaiveDate, UsagePeriod> = BTreeMap::new();
    let mut unavailable = 0;
    for result in results {
        if result.id != point.id {
            return Err(invalid("response contains an unrequested metering point"));
        }
        if !result.success {
            if result.error_code == 30015 {
                unavailable += 1;
                continue;
            }
            return Err(FetchError::Upstream(format!(
                "metering point {}: API error {}",
                point.id, result.error_code
            )));
        }
        let document = result
            .document
            .ok_or_else(|| invalid("missing market document"))?;
        let series = document
            .series
            .ok_or_else(|| invalid("missing time series"))?;
        if series.is_empty() {
            return Err(invalid(
                "empty time series; expected explicit no-data result",
            ));
        }
        for series in series {
            if series.evaluation_point.id.name != point.id {
                return Err(invalid("time series metering point ID mismatch"));
            }
            if series.unit != "KWH" && series.unit != "kWh" {
                return Err(invalid("expected kWh measurement unit"));
            }
            for period in series.periods {
                if period.resolution != "P1D" {
                    return Err(invalid("expected daily (P1D) resolution"));
                }
                let start = DateTime::parse_from_rfc3339(&period.interval.start)
                    .map_err(|_| invalid("invalid period start"))?
                    .with_timezone(&Copenhagen);
                let end = DateTime::parse_from_rfc3339(&period.interval.end)
                    .map_err(|_| invalid("invalid period end"))?
                    .with_timezone(&Copenhagen);
                if start.time() != chrono::NaiveTime::MIN
                    || end.time() != chrono::NaiveTime::MIN
                    || start >= end
                {
                    return Err(invalid(
                        "daily intervals must start and end at Danish midnight",
                    ));
                }
                for p in period.points {
                    let position: u32 = p
                        .position
                        .parse()
                        .map_err(|_| invalid("invalid point position"))?;
                    if position == 0 {
                        return Err(invalid("positions are one-based"));
                    }
                    // Add calendar days, not 24h: Danish days can be 23 or 25 hours.
                    let day = start
                        .date_naive()
                        .checked_add_signed(Duration::days(i64::from(position) - 1))
                        .ok_or_else(|| invalid("position overflow"))?;
                    if day >= end.date_naive() || day < from || day >= to {
                        return Err(invalid("point outside requested or declared interval"));
                    }
                    let missing = p.quality == "A02";
                    if !matches!(p.quality.as_str(), "A01" | "A02" | "A03" | "A04" | "A05") {
                        return Err(invalid("unknown quantity quality"));
                    }
                    let amount = if missing || (p.quality == "A05" && p.quantity.is_none()) {
                        None
                    } else {
                        Some(milli_kwh(p.quantity.as_deref().ok_or_else(|| {
                            invalid("missing quantity without A02 indicator")
                        })?)?)
                    };
                    let complete = amount.is_some() && p.quality != "A05";
                    let row = UsagePeriod {
                        metering_point_id: point.id.clone(),
                        direction: point.direction,
                        granularity: Granularity::Day,
                        period_start: day.to_string(),
                        period_end: (day + Duration::days(1)).to_string(),
                        milli_kwh: amount,
                        quality: p.quality,
                        complete,
                    };
                    if let Some(previous) = days.insert(day, row.clone())
                        && (previous.milli_kwh != row.milli_kwh || previous.quality != row.quality)
                    {
                        return Err(invalid(
                            "conflicting daily points in repeated access periods",
                        ));
                    }
                }
            }
        }
    }
    if days.is_empty() && unavailable == 0 {
        return Err(invalid("successful response contains no daily points"));
    }
    Ok((days.into_values().collect(), unavailable))
}

pub fn aggregate(days: Vec<UsagePeriod>) -> Result<Vec<UsagePeriod>, FetchError> {
    let mut groups: BTreeMap<(u8, NaiveDate), Vec<&UsagePeriod>> = BTreeMap::new();
    for row in &days {
        if row.granularity != Granularity::Day {
            return Err(invalid("storage returned a non-daily row"));
        }
        let day = date(&row.period_start)?;
        if date(&row.period_end)? != day + Duration::days(1) {
            return Err(invalid("invalid stored daily interval"));
        }
        for (kind, start) in [
            (
                1,
                day - Duration::days(i64::from(day.weekday().num_days_from_monday())),
            ),
            (2, day.with_day(1).unwrap()),
            (3, NaiveDate::from_ymd_opt(day.year(), 1, 1).unwrap()),
        ] {
            groups.entry((kind, start)).or_default().push(row);
        }
    }
    let mut output = Vec::new();
    for ((kind, start), rows) in groups {
        let (granularity, end) = match kind {
            1 => (Granularity::Week, start + Duration::days(7)),
            2 => (
                Granularity::Month,
                (start + Duration::days(32)).with_day(1).unwrap(),
            ),
            _ => (
                Granularity::Year,
                NaiveDate::from_ymd_opt(start.year() + 1, 1, 1).unwrap(),
            ),
        };
        let mut sum = 0_i64;
        let mut missing = false;
        let mut complete = rows.len() as i64 == (end - start).num_days();
        let mut qualities = BTreeSet::new();
        for row in &rows {
            if row.metering_point_id != rows[0].metering_point_id
                || row.direction != rows[0].direction
            {
                return Err(invalid(
                    "aggregate must contain exactly one metering point and direction",
                ));
            }
            match row.milli_kwh {
                Some(value) if value >= 0 => {
                    sum = sum
                        .checked_add(value)
                        .ok_or_else(|| invalid("aggregate overflow"))?
                }
                Some(_) => return Err(invalid("negative stored quantity")),
                None => missing = true,
            }
            complete &= row.complete;
            qualities.insert(row.quality.as_str());
        }
        output.push(UsagePeriod {
            metering_point_id: rows[0].metering_point_id.clone(),
            direction: rows[0].direction,
            granularity,
            period_start: start.to_string(),
            period_end: end.to_string(),
            milli_kwh: (!missing).then_some(sum),
            quality: if !complete {
                "A05".into()
            } else if qualities.contains("A03") {
                "A03".into()
            } else if qualities.len() == 1 {
                rows[0].quality.clone()
            } else {
                "A05".into()
            },
            complete,
        });
    }
    output.extend(days);
    Ok(output)
}

pub fn today() -> NaiveDate {
    Copenhagen
        .from_utc_datetime(&chrono::Utc::now().naive_utc())
        .date_naive()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn address() -> Address {
        Address {
            street_name: " Ågade ".into(),
            building_number: "12".into(),
            postcode: "2200".into(),
            floor: None,
            door: None,
            city: None,
        }
    }

    fn api_point(floor: &str) -> ApiPoint {
        serde_json::from_value(serde_json::json!({
            "meteringPointId":"571313180000000001","typeOfMP":"E17","hasRelation":true,
            "streetName":"ågade","buildingNumber":"12","postcode":"2200","floorId":floor,
            "childMeteringPoints":[{"meteringPointId":"571313180000000002","typeOfMP":"D06"}]
        }))
        .unwrap()
    }

    fn point() -> MeteringPoint {
        select(&address(), vec![api_point("1")])
            .unwrap()
            .remove(0)
            .point
    }

    fn day(value: &str, quantity: Option<i64>) -> UsagePeriod {
        UsagePeriod {
            metering_point_id: point().id,
            direction: Direction::Consumption,
            granularity: Granularity::Day,
            period_start: value.into(),
            period_end: (date(value).unwrap() + Duration::days(1)).to_string(),
            milli_kwh: quantity,
            quality: if quantity.is_some() { "A04" } else { "A02" }.into(),
            complete: quantity.is_some(),
        }
    }

    #[test]
    fn matches_danish_address_and_separates_grid_export() {
        let points = select(&address(), vec![api_point("1")]).unwrap();
        assert_eq!(points.len(), 2);
        assert_eq!(points[1].point.direction, Direction::GridExport);
        assert!(matches!(
            select(&address(), vec![api_point("1"), api_point("2")]),
            Err(FetchError::AmbiguousAddress(_))
        ));
        let mut wanted = address();
        wanted.floor = Some("2".into());
        assert_eq!(
            select(&wanted, vec![api_point("1"), api_point("2")])
                .unwrap()
                .len(),
            2
        );
    }

    #[test]
    fn rejects_unlinked_and_missing_address() {
        let mut p = api_point("1");
        p.has_relation = false;
        assert!(matches!(
            select(&address(), vec![p]),
            Err(FetchError::NoRelation(_))
        ));
        assert!(matches!(
            select(&address(), vec![]),
            Err(FetchError::AddressNotFound(_))
        ));
    }

    #[test]
    fn decimal_quantities_are_exact_and_checked() {
        assert_eq!(milli_kwh("12.003").unwrap(), 12003);
        assert_eq!(milli_kwh("0.1").unwrap(), 100);
        for value in ["-1", "NaN", "1.0001", "9223372036854775807", "1,1"] {
            assert!(milli_kwh(value).is_err(), "{value}");
        }
    }

    #[test]
    fn aggregates_iso_weeks_missing_and_partial_periods() {
        let output = aggregate(vec![
            day("2020-12-31", Some(100)),
            day("2021-01-01", Some(200)),
        ])
        .unwrap();
        let week = output
            .iter()
            .find(|r| r.granularity == Granularity::Week)
            .unwrap();
        assert_eq!(week.period_start, "2020-12-28");
        assert_eq!(week.milli_kwh, Some(300));
        assert!(!week.complete);
        assert_eq!(
            output
                .iter()
                .filter(|r| r.granularity == Granularity::Year)
                .count(),
            2
        );
        let output = aggregate(vec![day("2026-01-01", None)]).unwrap();
        assert!(output.iter().all(|r| r.milli_kwh.is_none()));
    }

    #[test]
    fn incremental_rollups_include_previous_days_and_corrections() {
        let mut history = BTreeMap::new();
        history.insert("2026-01-01", day("2026-01-01", Some(100)));
        history.insert("2026-01-02", day("2026-01-02", Some(100)));
        history.insert("2026-01-02", day("2026-01-02", Some(200)));
        history.insert("2026-01-03", day("2026-01-03", Some(300)));
        let rows = aggregate(history.into_values().collect()).unwrap();
        assert_eq!(
            rows.iter()
                .find(|r| r.granularity == Granularity::Month)
                .unwrap()
                .milli_kwh,
            Some(600)
        );
    }

    #[test]
    fn parses_daily_calendar_positions_across_dst() {
        let point = point();
        let response = serde_json::json!([{
            "success":true,"errorCode":0,"id":point.id,
            "MyEnergyData_MarketDocument":{"TimeSeries":[{
                "measurement_Unit.name":"KWH",
                "MarketEvaluationPoint":{"mRID":{"name":point.id}},
                "Period":[{"resolution":"P1D","timeInterval":{
                    "start":"2026-03-27T23:00:00Z","end":"2026-03-30T22:00:00Z"},
                    "Point":[
                        {"position":"1","out_Quantity.quantity":"1.000","out_Quantity.quality":"A04"},
                        {"position":"2","out_Quantity.quantity":"2.000","out_Quantity.quality":"A04"},
                        {"position":"3","out_Quantity.quantity":null,"out_Quantity.quality":"A02"}
                    ]}]
            }]}
        }]);
        let results = serde_json::from_value(response.clone()).unwrap();
        let (rows, _) = parse_days(
            &point,
            results,
            date("2026-03-28").unwrap(),
            date("2026-03-31").unwrap(),
        )
        .unwrap();
        assert_eq!(rows[2].period_start, "2026-03-30");
        assert_eq!(rows[2].milli_kwh, None);
        let mut wrong = response;
        wrong[0]["MyEnergyData_MarketDocument"]["TimeSeries"][0]["measurement_Unit.name"] =
            "MWH".into();
        assert!(
            parse_days(
                &point,
                serde_json::from_value(wrong).unwrap(),
                date("2026-03-28").unwrap(),
                date("2026-03-31").unwrap()
            )
            .is_err()
        );
    }

    #[test]
    fn leap_month_is_complete_and_sums_exactly() {
        let rows = (1..=29)
            .map(|d| day(&format!("2024-02-{d:02}"), Some(1)))
            .collect();
        let output = aggregate(rows).unwrap();
        let month = output
            .iter()
            .find(|r| r.granularity == Granularity::Month)
            .unwrap();
        assert_eq!(month.milli_kwh, Some(29));
        assert_eq!(month.period_end, "2024-03-01");
        assert!(month.complete);
        assert_eq!(month.quality, "A04");
    }

    #[test]
    fn reports_no_data_but_rejects_unexpected_ids_and_errors() {
        let point = point();
        let response = |id: &str, code| SeriesResult {
            success: false,
            error_code: code,
            id: id.into(),
            document: None,
        };
        let from = date("2026-01-01").unwrap();
        let to = date("2026-01-02").unwrap();
        let (rows, unavailable) =
            parse_days(&point, vec![response(&point.id, 30015)], from, to).unwrap();
        assert!(rows.is_empty());
        assert_eq!(unavailable, 1);
        assert!(parse_days(&point, vec![response("unrequested", 30015)], from, to).is_err());
        assert!(parse_days(&point, vec![response(&point.id, 30010)], from, to).is_err());
        assert!(parse_days(&point, vec![], from, to).is_err());
    }
}
