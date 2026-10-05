wit_bindgen::generate!({
    path: "wit",
    world: "nearby-weather-stations",
});

use exports::pwrusage::weather::stations::{Error, Guest, NearbyStation, Position, WeatherStation};

struct Component;

impl Guest for Component {
    fn get_nearby_weather_stations(
        my_position: Position,
        stations: Vec<WeatherStation>,
    ) -> Result<Vec<NearbyStation>, Error> {
        nearest_weather_stations(my_position, stations)
    }
}

#[cfg(target_family = "wasm")]
export!(Component);

const EARTH_RADIUS_KM: f64 = 6_371.008_8;
const MAX_NEARBY_STATIONS: usize = 3;

fn nearest_weather_stations(
    my_position: Position,
    stations: Vec<WeatherStation>,
) -> Result<Vec<NearbyStation>, Error> {
    if !is_valid_position(&my_position) {
        return Err(Error::InvalidPosition(
            "latitude must be between -90 and 90 and longitude between -180 and 180".to_string(),
        ));
    }

    let mut nearby: Vec<_> = stations
        .into_iter()
        .filter(|station| is_valid_position(&station.position))
        .map(|station| NearbyStation {
            distance_km: haversine_km(&my_position, &station.position),
            station,
        })
        .collect();

    nearby.sort_by(|left, right| left.distance_km.total_cmp(&right.distance_km));
    nearby.truncate(MAX_NEARBY_STATIONS);
    Ok(nearby)
}

fn is_valid_position(position: &Position) -> bool {
    position.latitude.is_finite()
        && position.longitude.is_finite()
        && (-90.0..=90.0).contains(&position.latitude)
        && (-180.0..=180.0).contains(&position.longitude)
}

fn haversine_km(from: &Position, to: &Position) -> f64 {
    let latitude_delta = (to.latitude - from.latitude).to_radians();
    let longitude_delta = (to.longitude - from.longitude).to_radians();
    let from_latitude = from.latitude.to_radians();
    let to_latitude = to.latitude.to_radians();

    let haversine = (latitude_delta / 2.0).sin().powi(2)
        + from_latitude.cos() * to_latitude.cos() * (longitude_delta / 2.0).sin().powi(2);
    let haversine = haversine.clamp(0.0, 1.0);
    let central_angle = 2.0 * haversine.sqrt().atan2((1.0 - haversine).sqrt());

    EARTH_RADIUS_KM * central_angle
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_three_nearest_stations_in_distance_order() {
        let result = nearest_weather_stations(
            position(0.0, 0.0),
            vec![
                station("fourth", 0.0, 4.0),
                station("second", 0.0, 2.0),
                station("nearest", 0.0, 1.0),
                station("third", 0.0, 3.0),
            ],
        )
        .expect("position should be valid");

        let ids: Vec<_> = result
            .iter()
            .map(|nearby| nearby.station.id.as_str())
            .collect();
        assert_eq!(ids, ["nearest", "second", "third"]);
        assert!(
            result
                .windows(2)
                .all(|pair| { pair[0].distance_km <= pair[1].distance_km })
        );
    }

    #[test]
    fn returns_all_stations_when_fewer_than_three_are_valid() {
        let result = nearest_weather_stations(
            position(52.52, 13.405),
            vec![station("valid", 52.5, 13.4), station("invalid", 91.0, 13.4)],
        )
        .expect("position should be valid");

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].station.id, "valid");
    }

    #[test]
    fn returns_empty_list_for_no_stations() {
        let result =
            nearest_weather_stations(position(0.0, 0.0), vec![]).expect("position should be valid");

        assert!(result.is_empty());
    }

    #[test]
    fn rejects_invalid_caller_position() {
        let result =
            nearest_weather_stations(position(f64::NAN, 0.0), vec![station("valid", 0.0, 0.0)]);

        assert!(matches!(result, Err(Error::InvalidPosition(_))));
    }

    #[test]
    fn calculates_known_distance() {
        let berlin = position(52.520_008, 13.404_954);
        let paris = position(48.856_613, 2.352_222);

        let distance = haversine_km(&berlin, &paris);

        assert!((distance - 877.46).abs() < 0.5);
    }

    #[test]
    fn keeps_input_order_when_distances_are_equal() {
        let result = nearest_weather_stations(
            position(0.0, 0.0),
            vec![station("east", 0.0, 1.0), station("west", 0.0, -1.0)],
        )
        .expect("position should be valid");

        assert_eq!(result[0].station.id, "east");
        assert_eq!(result[1].station.id, "west");
    }

    fn position(latitude: f64, longitude: f64) -> Position {
        Position {
            latitude,
            longitude,
        }
    }

    fn station(id: &str, latitude: f64, longitude: f64) -> WeatherStation {
        WeatherStation {
            id: id.to_string(),
            position: position(latitude, longitude),
        }
    }
}
