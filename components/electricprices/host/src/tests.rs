use super::*;
use bindings::pwrusage::electric_prices::types::{FetchError, FetchSummary, PriceKind};
use chrono::{Duration, TimeZone, Timelike, Utc};
use http_body_util::BodyExt;
use serde_json::json;

type HttpResult = Result<
    (
        http::Response<wasmtime_wasi_http::WasiBody>,
        Box<dyn Future<Output = Result<(), wasmtime_wasi_http::Error>> + Send>,
    ),
    wasmtime_wasi_http::Error,
>;

#[derive(Clone)]
pub(super) struct MockHttp {
    corrected: bool,
    fail_upcoming: bool,
    requests: Arc<std::sync::Mutex<Vec<String>>>,
}

fn stamp(value: chrono::DateTime<Utc>) -> String {
    value.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

fn today() -> chrono::DateTime<Utc> {
    chrono_tz::Europe::Copenhagen
        .from_local_datetime(
            &Utc::now()
                .with_timezone(&chrono_tz::Europe::Copenhagen)
                .date_naive()
                .and_hms_opt(0, 0, 0)
                .unwrap(),
        )
        .single()
        .unwrap()
        .with_timezone(&Utc)
}

impl MockHttp {
    pub(super) fn send(
        &self,
        request: http::Request<wasmtime_wasi_http::WasiBody>,
    ) -> Box<dyn Future<Output = HttpResult> + Send> {
        let mock = self.clone();
        Box::new(async move {
            assert_eq!(
                request.headers()["auth-token"],
                "fake-electricitymaps-token"
            );
            assert_eq!(request.method(), http::Method::GET);
            let path = request.uri().path();
            mock.requests
                .lock()
                .unwrap()
                .push(request.uri().to_string());
            let mut status = 200;
            let payload = match path {
                "/v3/zone" => {
                    assert_eq!(request.uri().query(), Some("lat=55.7&lon=12.5"));
                    json!({"zoneKey":"DK-DK2"})
                }
                "/v3/price-day-ahead/latest" => {
                    assert_eq!(
                        request.uri().query(),
                        Some("zone=DK-DK2&temporalGranularity=hourly")
                    );
                    let hour = Utc::now()
                        .with_minute(0)
                        .unwrap()
                        .with_second(0)
                        .unwrap()
                        .with_nanosecond(0)
                        .unwrap();
                    json!({"zone":"DK-DK2","datetime":stamp(hour),"value":100.0,"unit":"EUR/MWh"})
                }
                "/v3/price-day-ahead/past-range" => {
                    let query = request.uri().query().unwrap();
                    let start = query
                        .split('&')
                        .find_map(|v| v.strip_prefix("start="))
                        .unwrap();
                    let end = query
                        .split('&')
                        .find_map(|v| v.strip_prefix("end="))
                        .unwrap();
                    let start = chrono::DateTime::parse_from_rfc3339(start)
                        .unwrap()
                        .with_timezone(&Utc);
                    let end = chrono::DateTime::parse_from_rfc3339(end)
                        .unwrap()
                        .with_timezone(&Utc);
                    assert!(end <= today());
                    assert!(end - start <= Duration::days(5));
                    let mut data = Vec::new();
                    let mut cursor = start;
                    while cursor < end {
                        data.push(json!({"datetime":stamp(cursor),"value":
                            if mock.corrected { -50.0 } else { 100.0 }, "unit":"EUR/MWh"}));
                        cursor += Duration::hours(1);
                    }
                    json!({"zone":"DK-DK2","temporalGranularity":"hourly","data":data})
                }
                "/v4/price-day-ahead/actual" => {
                    if mock.fail_upcoming {
                        status = 403;
                    }
                    let now = Utc::now();
                    let current = now
                        .with_minute(0)
                        .unwrap()
                        .with_second(0)
                        .unwrap()
                        .with_nanosecond(0)
                        .unwrap();
                    json!({"zone":"DK-DK2","temporalGranularity":"hourly","data":[
                        {"datetime":stamp(current),"value":100.0,"unit":"EUR/MWh"},
                        {"datetime":stamp(current + Duration::hours(1)),"value":-20.0,"unit":"EUR/MWh"},
                        {"datetime":stamp(current + Duration::hours(2)),"value":null,"unit":"EUR/MWh"}
                    ]})
                }
                _ => panic!("unexpected API endpoint {path}"),
            };
            Ok((
                http::Response::builder()
                    .status(status)
                    .body(
                        http_body_util::Full::new(bytes::Bytes::from(payload.to_string()))
                            .map_err(|e| match e {})
                            .boxed_unsync(),
                    )
                    .unwrap(),
                Box::new(async { Ok(()) }) as Box<dyn Future<Output = _> + Send>,
            ))
        })
    }
}

#[test]
fn restricts_http_to_electricitymaps_price_routes() {
    assert!(allowed_uri(
        &"https://api.electricitymaps.com/v3/zone?lat=55&lon=12"
            .parse()
            .unwrap()
    ));
    assert!(allowed_uri(
        &"https://api.electricitymaps.com/v4/price-day-ahead/actual"
            .parse()
            .unwrap()
    ));
    for uri in [
        "http://api.electricitymaps.com/v3/zone",
        "https://api.electricitymaps.com:8443/v3/zone",
        "https://api.electricitymaps.com.evil.invalid/v3/zone",
        "https://127.0.0.1/v3/zone",
        "https://api.electricitymaps.com/v3/other",
        "https://api.electricitymaps.com/v3/../secret",
    ] {
        assert!(!allowed_uri(&uri.parse().unwrap()), "{uri}");
    }
}

#[test]
fn cli_requires_either_zone_or_complete_coordinates() {
    let base = [
        "host",
        "--component",
        "test.wasm",
        "--database-url",
        "postgresql://localhost/test",
        "--provider-name",
        "Example",
        "--markup",
        "0.1",
    ];
    for extra in [
        vec![],
        vec!["--lat", "55"],
        vec!["--lon", "12"],
        vec!["--zone", "DK-DK2", "--lat", "55", "--lon", "12"],
    ] {
        assert!(Args::try_parse_from(base.into_iter().chain(extra)).is_err());
    }
    for extra in [
        vec!["--zone", "DK-DK2"],
        vec!["--lat", "55", "--lon", "-12"],
    ] {
        assert!(Args::try_parse_from(base.into_iter().chain(extra)).is_ok());
    }
}

async fn invoke(
    database: Arc<Mutex<tokio_postgres::Client>>,
    component: &Component,
    engine: &Engine,
    position: Position,
    corrected: bool,
    fail_upcoming: bool,
    with_token: bool,
) -> Result<(Result<FetchSummary, FetchError>, Vec<String>)> {
    let requests = Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut wasi = WasiCtxBuilder::new();
    wasi.inherit_stderr().env("EUR_DKK_RATE", "7.46");
    if with_token {
        wasi.env("ELECTRICITYMAPS_API_TOKEN", "fake-electricitymaps-token");
    }
    let state = State {
        wasi: wasi.build(),
        http: WasiHttpCtx::new(),
        hooks: HttpPolicy {
            mock: Some(MockHttp {
                corrected,
                fail_upcoming,
                requests: Arc::clone(&requests),
            }),
        },
        table: ResourceTable::new(),
        database,
    };
    let mut store = Store::new(engine, state);
    let instance =
        bindings::ElectricPrices::instantiate_async(&mut store, component, &linker(engine)?)
            .await?;
    let provider = Provider {
        name: "Example supplier".into(),
        markup_per_kwh: 0.1,
        markup_currency: "DKK".into(),
        include_vat: true,
    };
    let history_from = (today()
        .with_timezone(&chrono_tz::Europe::Copenhagen)
        .date_naive()
        - Duration::days(2))
    .to_string();
    let result = store
        .run_concurrent(async |accessor| {
            instance
                .pwrusage_electric_prices_fetcher()
                .call_fetch(accessor, provider, position, Some(history_from), None)
                .await
        })
        .await??;
    let requests = requests.lock().unwrap().clone();
    Ok((result, requests))
}

#[tokio::test]
#[ignore = "requires TEST_COMPONENT_PATH and TEST_DATABASE_URL pointing to an empty disposable database"]
async fn compiled_component_persists_corrects_and_is_atomic() -> Result<()> {
    let url = std::env::var("TEST_DATABASE_URL").context("TEST_DATABASE_URL required")?;
    let path = std::env::var("TEST_COMPONENT_PATH").context("TEST_COMPONENT_PATH required")?;
    let database = database::connect(&url).await?;
    let empty: bool = database
        .query_one(
            "SELECT NOT EXISTS (SELECT FROM information_schema.tables
         WHERE table_schema='public' AND table_name='electricity_prices')",
            &[],
        )
        .await?
        .get(0);
    anyhow::ensure!(empty, "use an empty disposable test database");
    database.batch_execute(database::MIGRATION).await?;
    database.batch_execute(database::MIGRATION).await?;
    let engine = engine()?;
    let component = Component::from_file(&engine, path)?;
    let database = Arc::new(Mutex::new(database));
    let (first, calls) = invoke(
        Arc::clone(&database),
        &component,
        &engine,
        Position::Coordinates(Coordinates {
            latitude: 55.7,
            longitude: 12.5,
        }),
        false,
        false,
        true,
    )
    .await?;
    let first = first.map_err(|e| anyhow::anyhow!("{e:?}"))?;
    assert!(first.history_records >= 47);
    assert_eq!(first.latest_records, 1);
    assert_eq!(first.forecast_records, 1);
    assert_eq!(first.unavailable_records, 1);
    assert_eq!(first.persisted_records, first.history_records + 2);
    assert!(calls[0].contains("/v3/zone?"));
    let count_before: i64 = database
        .lock()
        .await
        .query_one("SELECT count(*) FROM electricity_prices", &[])
        .await?
        .get(0);
    let (second, calls) = invoke(
        Arc::clone(&database),
        &component,
        &engine,
        Position::Zone("DK-DK2".into()),
        true,
        false,
        true,
    )
    .await?;
    let second = second.map_err(|e| anyhow::anyhow!("{e:?}"))?;
    assert!(!calls.iter().any(|uri| uri.contains("/v3/zone?")));
    assert_eq!(second.persisted_records, first.persisted_records);
    let client = database.lock().await;
    let row = client
        .query_one(
            "SELECT count(*), min(total_dkk_kwh) FROM electricity_prices",
            &[],
        )
        .await?;
    assert_eq!(row.get::<_, i64>(0), count_before);
    assert!((row.get::<_, f64>(1) + 0.34125).abs() < 1e-12);
    let history = database::last_history(&client, "DK-DK2", "Example supplier")
        .await?
        .unwrap();
    assert!(chrono::DateTime::parse_from_rfc3339(&history)? < today());
    assert!(
        database::last_history(&client, "DK-DK2", "Other supplier")
            .await?
            .is_none()
    );
    let days = client
        .query("SELECT available_hours FROM daily_prices", &[])
        .await?;
    assert_eq!(
        days.iter().map(|r| r.get::<_, i64>(0)).sum::<i64>(),
        count_before
    );
    drop(client);

    let (failed, _) = invoke(
        Arc::clone(&database),
        &component,
        &engine,
        Position::Zone("DK-DK2".into()),
        false,
        true,
        true,
    )
    .await?;
    assert!(matches!(failed, Err(FetchError::Auth(_))));
    let min: f64 = database
        .lock()
        .await
        .query_one("SELECT min(total_dkk_kwh) FROM electricity_prices", &[])
        .await?
        .get(0);
    assert!(
        (min + 0.34125).abs() < 1e-12,
        "failed fetch must not persist any earlier response"
    );
    for (position, with_token) in [
        (Position::Zone("DK-DK2".into()), false),
        (
            Position::Coordinates(Coordinates {
                latitude: 91.0,
                longitude: 0.0,
            }),
            true,
        ),
    ] {
        let (failed, calls) = invoke(
            Arc::clone(&database),
            &component,
            &engine,
            position,
            false,
            false,
            with_token,
        )
        .await?;
        assert!(matches!(
            failed,
            Err(FetchError::Config(_) | FetchError::InvalidPosition(_))
        ));
        assert!(calls.is_empty());
    }

    // A later upcoming snapshot may not replace a corrected historical value.
    let mut client = database.lock().await;
    let provider = Provider {
        name: "Example supplier".into(),
        markup_per_kwh: 0.1,
        markup_currency: "DKK".into(),
        include_vat: true,
    };
    let point = PricePoint {
        zone: "DK-DK2".into(),
        datetime: history,
        kind: PriceKind::Forecast,
        raw_value: 100.0,
        raw_unit: "EUR/MWh".into(),
        eur_dkk_rate: 7.46,
        spot_dkk_kwh: 0.746,
        markup_dkk_kwh: 0.1,
        total_dkk_kwh: 1.0575,
    };
    assert_eq!(
        database::persist(&mut client, &provider, &[point]).await?,
        0
    );
    Ok(())
}
