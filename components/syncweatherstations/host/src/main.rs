mod database;
mod http;

use std::{path::PathBuf, sync::Arc, time::Duration};

use anyhow::Result;
use clap::Parser;
use tokio::sync::Mutex;
use wasmtime::{
    Config, Engine, Store,
    component::{Component, HasSelf, Linker, ResourceTable},
};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

mod bindings {
    wasmtime::component::bindgen!({
        path: "../wit",
        world: "sync-weather-stations",
        imports: { default: async },
        exports: { default: async },
    });
}

use bindings::pwrusage::weather_sync::{
    storage::{Host as StorageHost, PersistSummary},
    transport::{Host as TransportHost, Response},
    types::StationVersion,
};

#[derive(Parser)]
struct Args {
    #[arg(long)]
    component: PathBuf,
    #[arg(long, env = "DATABASE_URL")]
    database_url: String,
}

struct State {
    wasi: WasiCtx,
    table: ResourceTable,
    http: reqwest::Client,
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

impl bindings::pwrusage::weather_sync::types::Host for State {}

impl TransportHost for State {}

impl bindings::pwrusage::weather_sync::transport::HostWithStore<State> for HasSelf<State> {
    async fn get(
        accessor: &wasmtime::component::Accessor<State, Self>,
        url: String,
    ) -> std::result::Result<Response, String> {
        let client = accessor.with(|mut access| access.get().http.clone());
        http::get(&client, &url)
            .await
            .map_err(|error| format!("{error:#}"))
    }

    async fn sleep_ms(_accessor: &wasmtime::component::Accessor<State, Self>, milliseconds: u64) {
        tokio::time::sleep(Duration::from_millis(milliseconds)).await;
    }
}

impl StorageHost for State {}

impl bindings::pwrusage::weather_sync::storage::HostWithStore<State> for HasSelf<State> {
    async fn persist(
        accessor: &wasmtime::component::Accessor<State, Self>,
        stations: Vec<StationVersion>,
    ) -> std::result::Result<PersistSummary, String> {
        let database = accessor.with(|mut access| Arc::clone(&access.get().database));
        let mut client = database.lock().await;
        database::persist(&mut client, &stations)
            .await
            .map_err(|error| format!("{error:#}"))
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let database = database::connect(&args.database_url).await?;
    database::migrate(&database).await?;

    let mut config = Config::new();
    config.wasm_component_model_async(true);
    config.wasm_component_model(true);
    config.concurrency_support(true);
    let engine = Engine::new(&config)
        .map_err(|error| anyhow::anyhow!("create Wasmtime engine: {error:#}"))?;
    let component = Component::from_file(&engine, &args.component)
        .map_err(|error| anyhow::anyhow!("load sync component: {error:#}"))?;
    let mut linker = Linker::new(&engine);
    wasmtime_wasi::p3::add_to_linker(&mut linker)
        .map_err(|error| anyhow::anyhow!("link WASIp3: {error:#}"))?;
    wasmtime_wasi::p2::add_to_linker_async(&mut linker)
        .map_err(|error| anyhow::anyhow!("link WASIp2: {error:#}"))?;
    bindings::SyncWeatherStations::add_to_linker::<_, HasSelf<_>>(&mut linker, |state| state)
        .map_err(|error| anyhow::anyhow!("link sync host capabilities: {error:#}"))?;

    let state = State {
        wasi: WasiCtxBuilder::new().build(),
        table: ResourceTable::new(),
        http: http::client()?,
        database: Arc::new(Mutex::new(database)),
    };
    let mut store = Store::new(&engine, state);
    let instance =
        bindings::SyncWeatherStations::instantiate_async(&mut store, &component, &linker)
            .await
            .map_err(|error| anyhow::anyhow!("instantiate sync component: {error:#}"))?;
    let result = store
        .run_concurrent(async |accessor| {
            instance
                .pwrusage_weather_sync_runner()
                .call_sync(accessor)
                .await
        })
        .await
        .map_err(|error| anyhow::anyhow!("invoke sync component: {error:#}"))?
        .map_err(|error| anyhow::anyhow!("invoke sync component: {error:#}"))?;

    match result {
        Ok(summary) => {
            println!(
                "pages={} received={} unique={} inserted={} changed={} unchanged={}",
                summary.pages,
                summary.received,
                summary.unique,
                summary.inserted,
                summary.changed,
                summary.unchanged
            );
            Ok(())
        }
        Err(error) => anyhow::bail!("synchronization failed: {error:?}"),
    }
}
