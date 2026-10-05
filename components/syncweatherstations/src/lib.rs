wit_bindgen::generate!({
    path: "wit",
    world: "sync-weather-stations",
});

mod model;
mod sync;

pub struct Component;

pub fn sync_weather_stations() -> Result<SyncSummary, SyncError> {
    sync::run()
}

impl Guest for Component {
    fn sync() -> Result<SyncSummary, SyncError> {
        sync_weather_stations()
    }
}

#[cfg(target_family = "wasm")]
export!(Component);
