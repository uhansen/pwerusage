mod storage;

use anyhow::Result;
use saveeye_telemetry::Envelope;
use spin_mqtt_sdk::{Metadata, Payload, mqtt_component};

#[mqtt_component]
async fn handle_message(message: Payload, metadata: Metadata) -> Result<()> {
    let envelope = match serde_json::from_slice::<Envelope>(&message) {
        Ok(envelope) => envelope,
        Err(error) => {
            eprintln!(
                "Sensor backend: invalid envelope on '{}': {error}",
                metadata.topic
            );
            return Ok(());
        }
    };
    if let Err(error) = envelope.validate() {
        eprintln!(
            "Sensor backend: rejected envelope on '{}': {error}",
            metadata.topic
        );
        return Ok(());
    }
    storage::persist(&envelope).await?;
    println!(
        "Stored from {}: {}",
        envelope.edge_id,
        envelope.reading.summary()
    );
    Ok(())
}
