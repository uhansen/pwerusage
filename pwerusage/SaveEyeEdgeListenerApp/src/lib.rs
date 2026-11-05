//! `SaveEyeEdgeListenerApp` forwards normalized SaveEye readings over MQTT.
//!
//! The SaveEye device is configured (in the SaveEye app) to publish local MQTT
//! telemetry to a broker. This component subscribes to that topic, validates
//! the JSON payload, filters by device serial when configured, derives a few
//! convenience values, and forwards a versioned reading to the server broker.
//!
//! The payload model and the derived values follow the official Home Assistant
//! integration for SaveEye devices (`saveeye/SaveEye-HomeAssistant`).

mod forwarding;

use anyhow::Result;
use chrono::Utc;
use spin_mqtt_sdk::{Metadata, Payload, mqtt_component};
use spin_sdk::variables;

use saveeye_telemetry::{Reading, Telemetry};

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
    forwarding::forward(reading.clone()).await?;

    println!("Forwarded: {}", reading.summary());
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
