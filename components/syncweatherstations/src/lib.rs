wit_bindgen::generate!({
    path: "wit",
    world: "sync-weather-stations",
});

mod model;
mod sync;

use crate::pwrusage::weather_sync::types::{SyncError, SyncSummary};

pub struct Component;

pub async fn sync_weather_stations() -> Result<SyncSummary, SyncError> {
    sync::run().await
}

impl exports::pwrusage::weather_sync::runner::Guest for Component {
    async fn sync() -> Result<SyncSummary, SyncError> {
        sync_weather_stations().await
    }
}

#[cfg(target_family = "wasm")]
export!(Component);
