mod storage;

use anyhow::Result;
use spin_mqtt_sdk::{Metadata, Payload, mqtt_component};

mod telemetry {
    wit_bindgen::generate!({
        path: "../../components/saveeye-telemetry/wit",
        world: "telemetry-client",
    });
}

use telemetry::pwrusage::saveeye::processor;

#[mqtt_component]
async fn handle_message(message: Payload, metadata: Metadata) -> Result<()> {
    let reading = match processor::validate(&message) {
        Ok(reading) => reading,
        Err(error) => {
            eprintln!(
                "Sensor backend: invalid envelope on '{}': {error}",
                metadata.topic
            );
            return Ok(());
        }
    };
    storage::persist(&reading).await?;
    println!("Stored from {}: {}", reading.edge_id, reading.summary);
    Ok(())
}
