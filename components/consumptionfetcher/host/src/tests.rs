use super::*;
use bindings::pwrusage::consumption_fetcher::types::{Direction, FetchSummary, Granularity};
use http_body_util::BodyExt;
use serde_json::{Value, json};

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
    fail_export: bool,
    calls: Arc<std::sync::Mutex<Vec<String>>>,
}

impl MockHttp {
    pub(super) fn send(
        &self,
        request: http::Request<wasmtime_wasi_http::WasiBody>,
    ) -> Box<dyn Future<Output = HttpResult> + Send> {
        let mock = self.clone();
        Box::new(async move {
            assert_eq!(request.headers()["api-version"], "1.0");
            let path = request.uri().path().to_owned();
            mock.calls.lock().unwrap().push(path.clone());
            let response = if path.ends_with("/token") {
                assert_eq!(
                    request.headers()["authorization"],
                    "Bearer fake-refresh-token"
                );
                json!({"result":"fake-access-token"})
            } else {
                assert_eq!(
                    request.headers()["authorization"],
                    "Bearer fake-access-token"
                );
                if path.ends_with("/meteringpoints/meteringpoints") {
                    assert_eq!(request.uri().query(), Some("includeAll=true"));
                    json!({"result":[{
                        "meteringPointId":"571313180000000001","typeOfMP":"E17","hasRelation":true,
                        "streetName":"Testvej","buildingNumber":"12","postcode":"2200",
                        "consumerStartDate":"2026-01-02T00:00:00",
                        "childMeteringPoints":[
                            {"meteringPointId":"571313180000000002","typeOfMP":"D01"},
                            {"meteringPointId":"571313180000000003","typeOfMP":"D06"}
                        ]
                    }]})
                } else {
                    assert_eq!(request.method(), http::Method::POST);
                    let body = request.into_body().collect().await.unwrap().to_bytes();
                    let body: Value = serde_json::from_slice(&body).unwrap();
                    let id = body["meteringPoints"]["meteringPoint"][0].as_str().unwrap();
                    assert_eq!(
                        body["meteringPoints"]["meteringPoint"]
                            .as_array()
                            .unwrap()
                            .len(),
                        1
                    );
                    if mock.fail_export && id.ends_with('3') {
                        return Ok((
                            http::Response::builder()
                                .status(500)
                                .body(
                                    http_body_util::Full::new(bytes::Bytes::new())
                                        .map_err(|e| match e {})
                                        .boxed_unsync(),
                                )
                                .unwrap(),
                            Box::new(async { Ok(()) }) as Box<dyn Future<Output = _> + Send>,
                        ));
                    }
                    let pieces: Vec<_> = path.split('/').collect();
                    assert_eq!(pieces[5], "2026-01-02");
                    assert_eq!(pieces[7], "Day");
                    let end_day: u32 = pieces[6][8..].parse().unwrap();
                    let multiplier: u32 = id[17..].parse().unwrap();
                    let points: Vec<_> = (1..end_day - 1).map(|position| json!({
                        "position":position.to_string(),
                        "out_Quantity.quantity": (if mock.corrected && position == 1 { 10 } else { position } * multiplier).to_string(),
                        "out_Quantity.quality":"A04"
                    })).collect();
                    json!({"result":[{
                        "success":true,"errorCode":0,"id":id,
                        "MyEnergyData_MarketDocument":{"TimeSeries":[{
                            "measurement_Unit.name":"KWH",
                            "MarketEvaluationPoint":{"mRID":{"name":id}},
                            "Period":[{"resolution":"P1D","timeInterval":{
                                "start":"2026-01-01T23:00:00Z",
                                "end":format!("2026-01-{:02}T23:00:00Z",end_day-1)
                            },"Point":points}]
                        }]}
                    }]})
                }
            };
            let bytes = bytes::Bytes::from(response.to_string());
            Ok((
                http::Response::new(
                    http_body_util::Full::new(bytes)
                        .map_err(|e| match e {})
                        .boxed_unsync(),
                ),
                Box::new(async { Ok(()) }) as Box<dyn Future<Output = _> + Send>,
            ))
        })
    }
}

#[test]
fn restricts_http_to_customer_api_https() {
    assert!(allowed_uri(
        &"https://api.eloverblik.dk/customerapi/api/token"
            .parse()
            .unwrap()
    ));
    for value in [
        "http://api.eloverblik.dk/customerapi/api/token",
        "https://api.eloverblik.dk.evil.invalid/customerapi/api/token",
        "https://api.eloverblik.dk:8443/customerapi/api/token",
        "https://127.0.0.1/customerapi/api/token",
        "https://api.eloverblik.dk/thirdpartyapi/api/token",
    ] {
        assert!(!allowed_uri(&value.parse().unwrap()), "{value}");
    }
}

async fn invoke(
    database: Arc<Mutex<tokio_postgres::Client>>,
    component: &Component,
    engine: &Engine,
    corrected: bool,
    fail_export: bool,
    end: &str,
) -> Result<Result<FetchSummary, bindings::pwrusage::consumption_fetcher::types::FetchError>> {
    let calls = Arc::new(std::sync::Mutex::new(Vec::new()));
    let state = State {
        wasi: WasiCtxBuilder::new()
            .inherit_stderr()
            .env("ELOVERBLIK_REFRESH_TOKEN", "fake-refresh-token")
            .build(),
        http: WasiHttpCtx::new(),
        hooks: HttpPolicy {
            mock: Some(MockHttp {
                corrected,
                fail_export,
                calls: Arc::clone(&calls),
            }),
        },
        table: ResourceTable::new(),
        database,
    };
    let mut store = Store::new(engine, state);
    let instance =
        bindings::ConsumptionFetcher::instantiate_async(&mut store, component, &linker(engine)?)
            .await?;
    let address = Address {
        street_name: "Testvej".into(),
        building_number: "12".into(),
        postcode: "2200".into(),
        floor: None,
        door: None,
        city: None,
    };
    let result = store
        .run_concurrent(async |accessor| {
            instance
                .pwrusage_consumption_fetcher_fetcher()
                .call_fetch(accessor, address, None, Some(end.into()))
                .await
        })
        .await??;
    assert_eq!(
        calls
            .lock()
            .unwrap()
            .iter()
            .filter(|p| p.ends_with("/token"))
            .count(),
        1
    );
    Ok(result)
}

#[tokio::test]
#[ignore = "requires TEST_COMPONENT_PATH and TEST_DATABASE_URL pointing to an empty disposable database"]
async fn compiled_component_persists_and_corrects_all_granularities() -> Result<()> {
    let url = std::env::var("TEST_DATABASE_URL").context("TEST_DATABASE_URL required")?;
    let path = std::env::var("TEST_COMPONENT_PATH").context("TEST_COMPONENT_PATH required")?;
    let database = database::connect(&url).await?;
    let empty: bool = database.query_one(
        "SELECT NOT EXISTS (SELECT FROM information_schema.tables WHERE table_schema='public' AND table_name='metering_points')",
        &[],
    ).await?.get(0);
    anyhow::ensure!(empty, "use an empty disposable test database");
    database.batch_execute(database::MIGRATION).await?;
    let engine = engine()?;
    let component = Component::from_file(&engine, path)?;
    let database = Arc::new(Mutex::new(database));
    let first = invoke(
        Arc::clone(&database),
        &component,
        &engine,
        false,
        false,
        "2026-01-04",
    )
    .await?
    .map_err(|e| anyhow::anyhow!("{e:?}"))?;
    assert_eq!(first.daily_records, 6);
    assert_eq!(first.metering_points, 3);
    let second = invoke(
        Arc::clone(&database),
        &component,
        &engine,
        true,
        false,
        "2026-01-05",
    )
    .await?
    .map_err(|e| anyhow::anyhow!("{e:?}"))?;
    assert_eq!(second.daily_records, 9);
    let client = database.lock().await;
    for (id, direction, total) in [
        ("571313180000000001", "consumption", 15000_i64),
        ("571313180000000002", "production", 30000_i64),
        ("571313180000000003", "grid-export", 45000_i64),
    ] {
        let rows = client
            .query(
                "SELECT milli_kwh, complete FROM usage_periods
             WHERE metering_point_id=$1 AND direction=$2 AND granularity <> 'day'",
                &[&id, &direction],
            )
            .await?;
        assert_eq!(rows.len(), 3);
        for row in rows {
            assert_eq!(row.get::<_, i64>(0), total);
            assert!(!row.get::<_, bool>(1));
        }
    }
    let count: i64 = client
        .query_one("SELECT count(*) FROM usage_periods", &[])
        .await?
        .get(0);
    assert_eq!(count, 18);
    drop(client);
    // Identical replay must not insert duplicates.
    invoke(
        Arc::clone(&database),
        &component,
        &engine,
        true,
        false,
        "2026-01-05",
    )
    .await?
    .map_err(|e| anyhow::anyhow!("{e:?}"))?;
    // Upstream failure after fetching earlier points must leave the entire run unpersisted.
    assert!(
        invoke(
            Arc::clone(&database),
            &component,
            &engine,
            false,
            true,
            "2026-01-05"
        )
        .await?
        .is_err()
    );
    let mut client = database.lock().await;
    assert_eq!(
        client
            .query_one("SELECT count(*) FROM usage_periods", &[])
            .await?
            .get::<_, i64>(0),
        count
    );
    assert_eq!(
        client
            .query_one(
                "SELECT milli_kwh FROM usage_periods WHERE metering_point_id='571313180000000001'
         AND granularity='month'",
                &[],
            )
            .await?
            .get::<_, i64>(0),
        15000
    );
    // A database error midway through persist must roll back all preceding writes.
    let point = MeteringPoint {
        id: "571313180000000001".into(),
        type_of_mp: "E17".into(),
        direction: Direction::Consumption,
        address: Address {
            street_name: "Changed".into(),
            building_number: "12".into(),
            postcode: "2200".into(),
            floor: None,
            door: None,
            city: None,
        },
    };
    let bad = UsagePeriod {
        metering_point_id: point.id.clone(),
        direction: Direction::Consumption,
        granularity: Granularity::Day,
        period_start: "2026-01-02".into(),
        period_end: "2026-01-03".into(),
        milli_kwh: Some(-1),
        quality: "A04".into(),
        complete: true,
    };
    assert!(
        database::persist(&mut client, &[point], &[bad])
            .await
            .is_err()
    );
    assert_eq!(
        client
            .query_one(
                "SELECT street_name FROM metering_points
        WHERE id='571313180000000001'",
                &[]
            )
            .await?
            .get::<_, String>(0),
        "Testvej"
    );
    Ok(())
}
