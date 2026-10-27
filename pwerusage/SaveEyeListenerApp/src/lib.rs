//! `SaveEyeListenerApp` — listens to SaveEye+ telemetry over MQTT.
//!
//! The SaveEye device is configured (in the SaveEye app) to publish local MQTT
//! telemetry to a broker. This component subscribes to that topic, validates
//! the JSON payload, filters by device serial when configured, derives a few
//! convenience values, and stores the latest reading in Spin's key-value store.
//!
//! Stored keys in the `default` store:
//!
//! * `saveeye:latest` — the most recent reading from any device.
//! * `saveeye:devices:<serial>` — the most recent reading for one device.
//!
//! The payload model and the derived values follow the official Home Assistant
//! integration for SaveEye devices (`saveeye/SaveEye-HomeAssistant`).

mod telemetry;

use anyhow::Result;
use chrono::Utc;
use spin_mqtt_sdk::{Metadata, Payload, mqtt_component};
use spin_sdk::key_value::Store;
use spin_sdk::variables;

use crate::telemetry::{Reading, Telemetry};

/// Key holding the most recent reading, regardless of device.
const LATEST_KEY: &str = "saveeye:latest";
/// Prefix for the per-device latest reading.
const DEVICE_KEY_PREFIX: &str = "saveeye:devices:";

/// Entry point invoked by the Spin MQTT trigger for every message on
/// `saveeye/telemetry` (see `spin.toml`).
///
/// Malformed messages are logged and acknowledged, so a bad payload cannot
/// poison the subscription or be redelivered forever.
#[mqtt_component]
async fn handle_message(message: Payload, metadata: Metadata) -> Result<()> {
    let payload = match std::str::from_utf8(&message) {
        Ok(payload) => payload,
        Err(error) => {
            eprintln!(
                "SaveEye: ignoring non-UTF-8 message on '{}': {error}",
                metadata.topic
            );
            return Ok(());
        }
    };

    let telemetry: Telemetry = match serde_json::from_str(payload) {
        Ok(telemetry) => telemetry,
        Err(error) => {
            eprintln!(
                "SaveEye: ignoring invalid telemetry on '{}': {error}",
                metadata.topic
            );
            return Ok(());
        }
    };

    if !is_wanted_device(telemetry.device_serial()).await? {
        return Ok(());
    }

    let reading = Reading::new(telemetry, Utc::now().to_rfc3339());
    let body = serde_json::to_vec(&reading)?;

    let store = Store::open_default().await?;
    store
        .set(
            format!("{DEVICE_KEY_PREFIX}{}", reading.telemetry.device_serial()),
            &body,
        )
        .await?;
    store.set(LATEST_KEY, &body).await?;

    println!("{}", reading.summary());
    Ok(())
}

/// Applies the optional `device_serial` variable.
///
/// An empty value (the default) accepts every SaveEye device publishing to the
/// topic. The comparison is case-insensitive because serials are displayed with
/// mixed case by the SaveEye app.
async fn is_wanted_device(serial: &str) -> Result<bool> {
    let wanted = match variables::get("device_serial").await {
        Ok(wanted) => wanted,
        // The variable is declared in spin.toml, but stay tolerant if a host
        // runs the component with a stripped-down manifest.
        Err(error) => {
            eprintln!("SaveEye: cannot read 'device_serial' variable: {error}");
            return Ok(true);
        }
    };
    let wanted = wanted.trim();

    if wanted.is_empty() || wanted.eq_ignore_ascii_case(serial) {
        Ok(true)
    } else {
        println!("SaveEye: ignoring telemetry from '{serial}' (expecting '{wanted}')");
        Ok(false)
    }
}
