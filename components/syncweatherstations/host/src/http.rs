use std::time::Duration;

use anyhow::{Context, Result, bail};
use reqwest::{Client, header::RETRY_AFTER};
use url::Url;

use crate::bindings::pwrusage::weather_sync::transport::Response;

const ALLOWED_HOST: &str = "opendataapi.dmi.dk";
const ALLOWED_PATH: &str = "/v2/metObs/collections/station/items";
const MAX_RESPONSE_BYTES: usize = 32 * 1024 * 1024;

pub fn client() -> Result<Client> {
    Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(60))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent("pwrusage-syncweatherstations/0.1")
        .build()
        .context("build HTTP client")
}

pub async fn get(client: &Client, value: &str) -> Result<Response> {
    let url = validate_url(value)?;
    let response = client.get(url).send().await.context("send DMI request")?;
    let status = response.status();
    let retry_after_ms = parse_retry_after(response.headers().get(RETRY_AFTER));
    let bytes = response.bytes().await.context("read DMI response")?;
    if bytes.len() > MAX_RESPONSE_BYTES {
        bail!("DMI response exceeds {MAX_RESPONSE_BYTES} bytes");
    }
    let body = String::from_utf8(bytes.to_vec()).context("DMI response is not UTF-8")?;
    Ok(Response {
        status: status.as_u16(),
        body,
        retry_after_ms,
    })
}

fn validate_url(value: &str) -> Result<Url> {
    let url = Url::parse(value).context("parse DMI URL")?;
    if url.scheme() != "https"
        || url.host_str() != Some(ALLOWED_HOST)
        || url.port().is_some()
        || url.path() != ALLOWED_PATH
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        bail!("refusing URL outside the DMI station endpoint");
    }
    Ok(url)
}

fn parse_retry_after(value: Option<&reqwest::header::HeaderValue>) -> Option<u64> {
    let seconds = value?.to_str().ok()?.parse::<u64>().ok()?;
    seconds.checked_mul(1_000)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restricts_requests_to_station_items() {
        assert!(
            validate_url(
                "https://opendataapi.dmi.dk/v2/metObs/collections/station/items?limit=1000"
            )
            .is_ok()
        );
        assert!(validate_url("https://example.com/").is_err());
        assert!(
            validate_url(
                "https://opendataapi.dmi.dk/v2/metObs/collections/observation/items?limit=1000"
            )
            .is_err()
        );
    }
}
