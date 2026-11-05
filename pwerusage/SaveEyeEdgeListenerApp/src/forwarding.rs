use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use futures::{FutureExt, select_biased};
use saveeye_telemetry::{Envelope, Reading};
use spin_sdk::{
    mqtt::{Connection, Qos},
    variables,
};

pub async fn forward(reading: Reading) -> Result<()> {
    let edge_id = variables::get("edge_id").await.context("read edge_id")?;
    let envelope = Envelope::new(edge_id.clone(), reading)?;
    let address = variables::get("upstream_address").await?;
    let username = variables::get("upstream_username").await?;
    let password = variables::get("upstream_password").await?;
    let topic = variables::get("upstream_topic").await?;
    // Per-invocation IDs avoid disconnecting another publisher on the same edge.
    let client_id = format!(
        "{edge_id}-{:016x}{:016x}",
        spin_sdk::wasip3::random::random::get_random_u64(),
        spin_sdk::wasip3::random::random::get_random_u64(),
    );
    let address = format!("{address}?client_id={client_id}");
    let publish = async {
        let connection = Connection::open(address, username, password, 30)
            .await
            .map_err(|_| {
                anyhow!("upstream MQTT open failed; check address, credentials and capability")
            })?;
        connection
            .publish(topic, serde_json::to_vec(&envelope)?, Qos::AtLeastOnce)
            .await
            .map_err(|_| {
                anyhow!("upstream MQTT publish failed; check broker connectivity and ACL")
            })?;
        Ok(())
    }
    .fuse();
    let timeout = spin_sdk::time::sleep(Duration::from_secs(5)).fuse();
    futures::pin_mut!(publish, timeout);
    select_biased! {
        result = publish => result,
        () = timeout => Err(anyhow!("upstream MQTT publish timed out after 5 seconds; reading not queued")),
    }
}
