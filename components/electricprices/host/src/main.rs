mod database;

use std::{path::PathBuf, sync::Arc};

use anyhow::{Context, Result};
use clap::Parser;
use tokio::sync::Mutex;
use wasmtime::{
    Config, Engine, Store,
    component::{Component, HasSelf, Linker, ResourceTable},
};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};
use wasmtime_wasi_http::{WasiHttpCtx, WasiHttpCtxView, WasiHttpHooks, WasiHttpView};

mod bindings {
    wasmtime::component::bindgen!({
        path: "../wit",
        world: "electric-prices",
        imports: { default: async },
        exports: { default: async },
    });
}

use bindings::pwrusage::electric_prices::types::{Coordinates, Position, PricePoint, Provider};

#[derive(Parser)]
#[command(about = "Fetch Electricity Maps day-ahead prices with supplier markup into PostgreSQL")]
struct Args {
    #[arg(long)]
    component: PathBuf,
    #[arg(long, env = "DATABASE_URL")]
    database_url: String,
    #[arg(long)]
    provider_name: String,
    #[arg(long, allow_hyphen_values = true)]
    markup: f64,
    #[arg(long, default_value = "DKK", value_parser = ["DKK", "EUR"])]
    markup_currency: String,
    #[arg(
        long,
        help = "Apply 25% VAT to spot price plus markup (default: no VAT)"
    )]
    vat: bool,
    #[arg(long, conflicts_with_all = ["lat", "lon"], required_unless_present = "lat")]
    zone: Option<String>,
    #[arg(
        long,
        requires = "lon",
        conflicts_with = "zone",
        allow_hyphen_values = true
    )]
    lat: Option<f64>,
    #[arg(
        long,
        requires = "lat",
        conflicts_with = "zone",
        allow_hyphen_values = true
    )]
    lon: Option<f64>,
    #[arg(long)]
    history_from: Option<String>,
    #[arg(
        long,
        help = "Exclusive end date, YYYY-MM-DD in Europe/Copenhagen (default: today)"
    )]
    history_to: Option<String>,
    #[arg(long, env = "EUR_DKK_RATE", default_value = "7.46")]
    eur_dkk_rate: f64,
}

#[derive(Default)]
struct HttpPolicy {
    #[cfg(test)]
    mock: Option<tests::MockHttp>,
}

fn allowed_uri(uri: &http::Uri) -> bool {
    uri.scheme_str() == Some("https")
        && uri.host() == Some("api.electricitymaps.com")
        && uri.port_u16().is_none_or(|port| port == 443)
        && matches!(
            uri.path(),
            "/v3/zone"
                | "/v3/price-day-ahead/latest"
                | "/v3/price-day-ahead/past-range"
                | "/v4/price-day-ahead/actual"
        )
}

impl WasiHttpHooks for HttpPolicy {
    fn send_request(
        &mut self,
        request: http::Request<wasmtime_wasi_http::WasiBody>,
        options: Option<wasmtime_wasi_http::RequestOptions>,
        _fut: Box<dyn Future<Output = Result<(), wasmtime_wasi_http::Error>> + Send>,
    ) -> Box<
        dyn Future<
                Output = Result<
                    (
                        http::Response<wasmtime_wasi_http::WasiBody>,
                        Box<dyn Future<Output = Result<(), wasmtime_wasi_http::Error>> + Send>,
                    ),
                    wasmtime_wasi_http::Error,
                >,
            > + Send,
    > {
        use http_body_util::BodyExt;
        if !allowed_uri(request.uri()) || request.method() != http::Method::GET {
            return Box::new(async { Err(wasmtime_wasi_http::Error::HttpRequestDenied) });
        }
        #[cfg(test)]
        if let Some(mock) = &self.mock {
            return mock.send(request);
        }
        Box::new(async move {
            let (response, io) = wasmtime_wasi_http::default_send_request(request, options).await?;
            Ok((
                response.map(BodyExt::boxed_unsync),
                Box::new(io) as Box<dyn Future<Output = _> + Send>,
            ))
        })
    }
}

struct State {
    wasi: WasiCtx,
    http: WasiHttpCtx,
    hooks: HttpPolicy,
    table: ResourceTable,
    database: Arc<Mutex<tokio_postgres::Client>>,
}

impl WasiView for State {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

impl WasiHttpView for State {
    fn http(&mut self) -> WasiHttpCtxView<'_> {
        WasiHttpCtxView {
            ctx: &mut self.http,
            table: &mut self.table,
            hooks: &mut self.hooks,
        }
    }
}

impl bindings::pwrusage::electric_prices::types::Host for State {}
impl bindings::pwrusage::electric_prices::storage::Host for State {}

impl bindings::pwrusage::electric_prices::storage::HostWithStore<State> for HasSelf<State> {
    async fn last_history(
        accessor: &wasmtime::component::Accessor<State, Self>,
        zone: String,
        provider_name: String,
    ) -> Result<Option<String>, String> {
        let database = accessor.with(|mut access| Arc::clone(&access.get().database));
        database::last_history(&*database.lock().await, &zone, &provider_name)
            .await
            .map_err(|e| format!("{e:#}"))
    }

    async fn persist(
        accessor: &wasmtime::component::Accessor<State, Self>,
        provider: Provider,
        points: Vec<PricePoint>,
    ) -> Result<u32, String> {
        let database = accessor.with(|mut access| Arc::clone(&access.get().database));
        database::persist(&mut *database.lock().await, &provider, &points)
            .await
            .map_err(|e| format!("{e:#}"))
    }
}

fn engine() -> Result<Engine> {
    let mut config = Config::new();
    config.wasm_component_model(true);
    config.wasm_component_model_async(true);
    config.concurrency_support(true);
    Engine::new(&config).map_err(|e| anyhow::anyhow!("create Wasmtime engine: {e:#}"))
}

fn linker(engine: &Engine) -> Result<Linker<State>> {
    let mut linker = Linker::new(engine);
    wasmtime_wasi::p3::add_to_linker(&mut linker)?;
    wasmtime_wasi::p2::add_to_linker_async(&mut linker)?;
    wasmtime_wasi_http::p3::add_to_linker(&mut linker)?;
    wasmtime_wasi_http::p2::add_only_http_to_linker_async(&mut linker)?;
    bindings::ElectricPrices::add_to_linker::<_, HasSelf<_>>(&mut linker, |state| state)?;
    Ok(linker)
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let token = std::env::var("ELECTRICITYMAPS_API_TOKEN")
        .context("ELECTRICITYMAPS_API_TOKEN is required")?;
    anyhow::ensure!(!token.trim().is_empty(), "API token is empty");
    anyhow::ensure!(
        args.eur_dkk_rate.is_finite() && args.eur_dkk_rate > 0.0,
        "EUR_DKK_RATE must be finite and positive"
    );
    anyhow::ensure!(args.markup.is_finite(), "markup must be finite");
    let position = match (args.zone, args.lat, args.lon) {
        (Some(zone), None, None) => Position::Zone(zone),
        (None, Some(latitude), Some(longitude)) => Position::Coordinates(Coordinates {
            latitude,
            longitude,
        }),
        _ => anyhow::bail!("provide --zone or both --lat and --lon"),
    };
    let database = database::connect(&args.database_url).await?;
    database
        .query_one("SELECT pg_advisory_lock($1)", &[&database::LOCK_ID])
        .await
        .context("lock electricprices sync")?;
    database
        .batch_execute(database::MIGRATION)
        .await
        .context("apply prices schema")?;
    let engine = engine()?;
    let component = Component::from_file(&engine, &args.component)
        .map_err(|e| anyhow::anyhow!("load electricprices component: {e:#}"))?;
    let linker = linker(&engine)?;
    let state = State {
        wasi: WasiCtxBuilder::new()
            .env("ELECTRICITYMAPS_API_TOKEN", &token)
            .env("EUR_DKK_RATE", args.eur_dkk_rate.to_string())
            .build(),
        http: WasiHttpCtx::new(),
        hooks: HttpPolicy::default(),
        table: ResourceTable::new(),
        database: Arc::new(Mutex::new(database)),
    };
    let mut store = Store::new(&engine, state);
    let instance =
        bindings::ElectricPrices::instantiate_async(&mut store, &component, &linker).await?;
    let provider = Provider {
        name: args.provider_name,
        markup_per_kwh: args.markup,
        markup_currency: args.markup_currency,
        include_vat: args.vat,
    };
    let result = store
        .run_concurrent(async |accessor| {
            instance
                .pwrusage_electric_prices_fetcher()
                .call_fetch(
                    accessor,
                    provider,
                    position,
                    args.history_from,
                    args.history_to,
                )
                .await
        })
        .await
        .map_err(|e| anyhow::anyhow!("invoke electricprices: {e:#}"))?
        .map_err(|e| anyhow::anyhow!("electricprices call: {e:#}"))?;
    match result {
        Ok(summary) => {
            println!(
                "zone={} history_records={} latest_records={} forecast_records={} unavailable_records={} persisted_records={}",
                summary.zone,
                summary.history_records,
                summary.latest_records,
                summary.forecast_records,
                summary.unavailable_records,
                summary.persisted_records,
            );
            Ok(())
        }
        Err(error) => anyhow::bail!("electricprices fetch failed: {error:?}"),
    }
}

#[cfg(test)]
mod tests;
