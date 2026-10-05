mod database;
mod http;

use std::{path::PathBuf, time::Duration};

use anyhow::Result;
use clap::Parser;
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
    database: tokio_postgres::Client,
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

impl TransportHost for State {
    async fn get(&mut self, url: String) -> std::result::Result<Response, String> {
        http::get(&self.http, &url)
            .await
            .map_err(|error| format!("{error:#}"))
    }

    async fn sleep_ms(&mut self, milliseconds: u64) {
        tokio::time::sleep(Duration::from_millis(milliseconds)).await;
    }
}

impl StorageHost for State {
    async fn persist(
        &mut self,
        stations: Vec<StationVersion>,
    ) -> std::result::Result<PersistSummary, String> {
        database::persist(&mut self.database, &stations)
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
    config.wasm_component_model(true);
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
        database,
    };
    let mut store = Store::new(&engine, state);
    let instance =
        bindings::SyncWeatherStations::instantiate_async(&mut store, &component, &linker)
            .await
            .map_err(|error| anyhow::anyhow!("instantiate sync component: {error:#}"))?;
    let result = instance
        .call_sync(&mut store)
        .await
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
