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

use anyhow::{Result, anyhow};
use chrono::Utc;
use spin_mqtt_sdk::{Metadata, Payload, mqtt_component};
use spin_sdk::variables;

mod telemetry {
    wit_bindgen::generate!({
        path: "../../components/saveeye-telemetry/wit",
        world: "telemetry-client",
    });
}

use telemetry::pwrusage::saveeye::processor::{self, Preparation, ProcessingError};

/// Entry point invoked by the Spin MQTT trigger for every message on
/// `saveeye/telemetry` (see `spin.toml`).
///
/// Malformed messages are logged and acknowledged, so a bad payload cannot
/// poison the subscription or be redelivered forever.
#[mqtt_component]
async fn handle_message(message: Payload, metadata: Metadata) -> Result<()> {
    let wanted = match variables::get("device_serial").await {
        Ok(wanted) => wanted,
        Err(error) => {
            eprintln!("SaveEye: cannot read 'device_serial' variable: {error}");
            String::new()
        }
    };
    let edge_id = variables::get("edge_id").await?;
    let prepared = match processor::prepare(&message, &Utc::now().to_rfc3339(), &edge_id, &wanted) {
        Ok(Preparation::Forward(prepared)) => prepared,
        Ok(Preparation::Filtered(reason)) => {
            println!("SaveEye: {reason}");
            return Ok(());
        }
        Err(ProcessingError::InvalidTelemetry(error)) => {
            eprintln!(
                "SaveEye: ignoring invalid telemetry on '{}': {error}",
                metadata.topic
            );
            return Ok(());
        }
        Err(error) => return Err(anyhow!("SaveEye processing failed: {error}")),
    };
    forwarding::forward(prepared.envelope, &edge_id).await?;
    println!("Forwarded: {}", prepared.summary);
    Ok(())
}
