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
        world: "consumption-fetcher",
        imports: { default: async },
        exports: { default: async },
    });
}

use bindings::pwrusage::consumption_fetcher::types::{Address, MeteringPoint, UsagePeriod};

#[derive(Parser)]
#[command(
    about = "Fetch Eloverblik household consumption and available production into PostgreSQL"
)]
struct Args {
    #[arg(long)]
    component: PathBuf,
    #[arg(long, env = "DATABASE_URL")]
    database_url: String,
    #[arg(long)]
    street: String,
    #[arg(long)]
    number: String,
    #[arg(long)]
    postcode: String,
    #[arg(long)]
    floor: Option<String>,
    #[arg(long)]
    door: Option<String>,
    #[arg(long)]
    city: Option<String>,
    #[arg(long)]
    date_from: Option<String>,
    #[arg(long)]
    date_to: Option<String>,
}

#[derive(Default)]
struct HttpPolicy {
    #[cfg(test)]
    mock: Option<tests::MockHttp>,
}

fn allowed_uri(uri: &http::Uri) -> bool {
    uri.scheme_str() == Some("https")
        && uri.host() == Some("api.eloverblik.dk")
        && uri.port_u16().is_none_or(|port| port == 443)
        && uri.path().starts_with("/customerapi/api/")
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
        if !allowed_uri(request.uri()) {
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

impl bindings::pwrusage::consumption_fetcher::types::Host for State {}
impl bindings::pwrusage::consumption_fetcher::storage::Host for State {}

impl bindings::pwrusage::consumption_fetcher::storage::HostWithStore<State> for HasSelf<State> {
    async fn load_days(
        accessor: &wasmtime::component::Accessor<State, Self>,
        id: String,
    ) -> Result<Vec<UsagePeriod>, String> {
        let database = accessor.with(|mut access| Arc::clone(&access.get().database));
        let client = database.lock().await;
        database::load_days(&client, &id)
            .await
            .map_err(|e| format!("{e:#}"))
    }

    async fn persist(
        accessor: &wasmtime::component::Accessor<State, Self>,
        points: Vec<MeteringPoint>,
        periods: Vec<UsagePeriod>,
    ) -> Result<u32, String> {
        let database = accessor.with(|mut access| Arc::clone(&access.get().database));
        let mut client = database.lock().await;
        database::persist(&mut client, &points, &periods)
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
    bindings::ConsumptionFetcher::add_to_linker::<_, HasSelf<_>>(&mut linker, |state| state)?;
    Ok(linker)
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let token = std::env::var("ELOVERBLIK_REFRESH_TOKEN")
        .context("ELOVERBLIK_REFRESH_TOKEN is required (portal refresh token)")?;
    anyhow::ensure!(!token.trim().is_empty(), "refresh token is empty");
    let database = database::connect(&args.database_url).await?;
    database
        .query_one("SELECT pg_advisory_lock($1)", &[&database::LOCK_ID])
        .await
        .context("lock consumption fetch (prevents concurrent stale rollups)")?;
    database
        .batch_execute(database::MIGRATION)
        .await
        .context("apply usage schema")?;
    let engine = engine()?;
    let component = Component::from_file(&engine, &args.component)
        .map_err(|e| anyhow::anyhow!("load fetcher component: {e:#}"))?;
    let linker = linker(&engine)?;
    let state = State {
        wasi: WasiCtxBuilder::new()
            .env("ELOVERBLIK_REFRESH_TOKEN", &token)
            .build(),
        http: WasiHttpCtx::new(),
        hooks: HttpPolicy::default(),
        table: ResourceTable::new(),
        database: Arc::new(Mutex::new(database)),
    };
    let mut store = Store::new(&engine, state);
    let instance =
        bindings::ConsumptionFetcher::instantiate_async(&mut store, &component, &linker).await?;
    let address = Address {
        street_name: args.street,
        building_number: args.number,
        postcode: args.postcode,
        floor: args.floor,
        door: args.door,
        city: args.city,
    };
    let result = store
        .run_concurrent(async |accessor| {
            instance
                .pwrusage_consumption_fetcher_fetcher()
                .call_fetch(accessor, address, args.date_from, args.date_to)
                .await
        })
        .await
        .map_err(|e| anyhow::anyhow!("invoke fetcher: {e:#}"))?
        .map_err(|e| anyhow::anyhow!("fetcher call: {e:#}"))?;
    // Releasing the session also releases the advisory lock on every error path.
    match result {
        Ok(summary) => {
            println!(
                "metering_points={} daily_records={} persisted_records={} unavailable_periods={}",
                summary.metering_points,
                summary.daily_records,
                summary.persisted_records,
                summary.unavailable_periods
            );
            Ok(())
        }
        Err(error) => anyhow::bail!("consumption fetch failed: {error:?}"),
    }
}

#[cfg(test)]
mod tests;
