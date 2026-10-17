use anyhow::{anyhow, Result};
use spin_cron_sdk::{cron_component, Metadata};

wit_bindgen::generate!({
    inline: r#"
        package pwrusage:weather-sync@0.1.0;

        interface types {
            record sync-summary {
                pages: u32,
                received: u64,
                unique: u64,
                inserted: u64,
                changed: u64,
                unchanged: u64,
            }

            variant sync-error {
                transport(string),
                upstream(string),
                invalid-data(string),
                storage(string),
                limit-exceeded(string),
            }
        }

        interface runner {
            use types.{sync-summary, sync-error};

            sync: async func() -> result<sync-summary, sync-error>;
        }

        world cron-runner {
            import runner;
        }
    "#,
});

#[cron_component]
async fn handle_cron_event(metadata: Metadata) -> Result<()> {
    let summary = pwrusage::weather_sync::runner::sync()
        .await
        .map_err(|error| anyhow!(format_sync_error(&error)))?;

    println!(
        "[{}] pages={} received={} unique={} inserted={} changed={} unchanged={}",
        metadata.timestamp,
        summary.pages,
        summary.received,
        summary.unique,
        summary.inserted,
        summary.changed,
        summary.unchanged
    );

    Ok(())
}

fn format_sync_error(error: &pwrusage::weather_sync::types::SyncError) -> String {
    match error {
        pwrusage::weather_sync::types::SyncError::Transport(message) => {
            format!("transport error: {message}")
        }
        pwrusage::weather_sync::types::SyncError::Upstream(message) => {
            format!("upstream error: {message}")
        }
        pwrusage::weather_sync::types::SyncError::InvalidData(message) => {
            format!("invalid data: {message}")
        }
        pwrusage::weather_sync::types::SyncError::Storage(message) => {
            format!("storage error: {message}")
        }
        pwrusage::weather_sync::types::SyncError::LimitExceeded(message) => {
            format!("limit exceeded: {message}")
        }
    }
}
