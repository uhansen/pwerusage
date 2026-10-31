use crate::FetchError;

#[cfg(target_os = "wasi")]
const MAX_BODY: usize = 16 * 1024 * 1024;
const BASE: &str = "https://api.eloverblik.dk/customerapi/api";

struct Response {
    status: u16,
    body: Vec<u8>,
    retry_after: Option<String>,
}

#[cfg(target_os = "wasi")]
fn transport(error: impl std::fmt::Debug) -> FetchError {
    FetchError::Upstream(format!("HTTP transport: {error:?}"))
}

pub(crate) async fn json<T: serde::de::DeserializeOwned>(
    path: &str,
    token: &str,
    body: Option<&str>,
) -> Result<T, FetchError> {
    for attempt in 0..4 {
        // Pace requests below the API's per-IP limit, including consecutive runs.
        sleep(600).await;
        let response = request(&format!("{BASE}{path}"), token, body).await?;
        match response.status {
            200 => {
                return serde_json::from_slice(&response.body).map_err(|_| {
                    FetchError::InvalidData(
                        "upstream response does not match the documented JSON schema".into(),
                    )
                });
            }
            401 | 403 => {
                return Err(FetchError::Auth(
                    "API rejected the token or authorization".into(),
                ));
            }
            429 | 503 if attempt < 3 => {
                let delay = retry_delay(response.retry_after.as_deref(), attempt)?;
                sleep(if path == "/token" {
                    delay.max(60_000)
                } else {
                    delay
                })
                .await;
            }
            429 => {
                return Err(FetchError::RateLimited(
                    "API rate limit exceeded after bounded retries".into(),
                ));
            }
            status => return Err(FetchError::Upstream(format!("API HTTP status {status}"))),
        }
    }
    unreachable!()
}

fn retry_delay(header: Option<&str>, attempt: u32) -> Result<u64, FetchError> {
    let millis = if let Some(value) = header {
        if let Ok(seconds) = value.parse::<u64>() {
            seconds
                .checked_mul(1000)
                .ok_or_else(|| FetchError::RateLimited("Retry-After overflow".into()))?
        } else {
            let timestamp = chrono::DateTime::parse_from_rfc2822(value)
                .map_err(|_| FetchError::Upstream("invalid Retry-After header".into()))?;
            (timestamp.with_timezone(&chrono::Utc) - chrono::Utc::now())
                .num_milliseconds()
                .max(0) as u64
        }
    } else {
        1000 * (1 << attempt)
    };
    if millis > 120_000 {
        return Err(FetchError::RateLimited(
            "Retry-After exceeds 120 seconds; retry this run later".into(),
        ));
    }
    Ok(millis)
}

#[cfg(all(target_os = "wasi", target_env = "p3"))]
async fn request(url: &str, token: &str, body: Option<&str>) -> Result<Response, FetchError> {
    use http_body_util::{BodyExt, Full};
    use wasip3::http_compat::{
        RequestOptionsExtension, http_from_wasi_response, http_into_wasi_request,
    };
    let options = wasip3::http::types::RequestOptions::new();
    options
        .set_connect_timeout(Some(30_000_000_000))
        .map_err(transport)?;
    options
        .set_first_byte_timeout(Some(60_000_000_000))
        .map_err(transport)?;
    options
        .set_between_bytes_timeout(Some(30_000_000_000))
        .map_err(transport)?;
    let mut request = http::Request::builder()
        .method(if body.is_some() { "POST" } else { "GET" })
        .uri(url)
        .header("authorization", format!("Bearer {token}"))
        .header("api-version", "1.0")
        .header("accept", "application/json")
        .header("content-type", "application/json")
        .body(Full::<bytes::Bytes>::new(
            body.unwrap_or("").as_bytes().to_vec().into(),
        ))
        .map_err(|_| FetchError::Config("invalid HTTP request headers".into()))?;
    request
        .extensions_mut()
        .insert(RequestOptionsExtension(options));
    let request = http_into_wasi_request(request).map_err(transport)?;
    let response = wasip3::http::client::send(request)
        .await
        .map_err(transport)?;
    let response = http_from_wasi_response(response).map_err(transport)?;
    let status = response.status().as_u16();
    let retry_after = response
        .headers()
        .get("retry-after")
        .map(|h| h.to_str().map(str::to_owned))
        .transpose()
        .map_err(transport)?;
    let mut body = std::pin::pin!(response.into_body());
    let mut bytes = Vec::new();
    while let Some(frame) = body.frame().await {
        let frame = frame.map_err(transport)?;
        if let Ok(data) = frame.into_data() {
            if bytes.len() + data.len() > MAX_BODY {
                return Err(FetchError::Upstream("HTTP response exceeds 16 MiB".into()));
            }
            bytes.extend_from_slice(&data);
        }
    }
    Ok(Response {
        status,
        body: bytes,
        retry_after,
    })
}

#[cfg(all(target_os = "wasi", not(target_env = "p3")))]
async fn request(url: &str, token: &str, body: Option<&str>) -> Result<Response, FetchError> {
    use wasi::{
        http::{outgoing_handler, types::*},
        io::streams::StreamError,
    };
    let fields = Fields::from_list(&[
        (
            "authorization".into(),
            format!("Bearer {token}").into_bytes(),
        ),
        ("api-version".into(), b"1.0".to_vec()),
        ("accept".into(), b"application/json".to_vec()),
        ("content-type".into(), b"application/json".to_vec()),
    ])
    .map_err(|_| FetchError::Config("invalid HTTP request headers".into()))?;
    let request = OutgoingRequest::new(fields);
    request
        .set_scheme(Some(&Scheme::Https))
        .map_err(transport)?;
    request
        .set_authority(Some("api.eloverblik.dk"))
        .map_err(transport)?;
    request
        .set_path_with_query(Some(
            url.strip_prefix("https://api.eloverblik.dk")
                .ok_or_else(|| FetchError::Config("invalid API URL".into()))?,
        ))
        .map_err(transport)?;
    request
        .set_method(&if body.is_some() {
            Method::Post
        } else {
            Method::Get
        })
        .map_err(transport)?;
    let outgoing = request.body().map_err(transport)?;
    let options = RequestOptions::new();
    options
        .set_connect_timeout(Some(30_000_000_000))
        .map_err(transport)?;
    options
        .set_first_byte_timeout(Some(60_000_000_000))
        .map_err(transport)?;
    options
        .set_between_bytes_timeout(Some(30_000_000_000))
        .map_err(transport)?;
    let pending = outgoing_handler::handle(request, Some(options)).map_err(transport)?;
    {
        let stream = outgoing.write().map_err(transport)?;
        stream
            .blocking_write_and_flush(body.unwrap_or("").as_bytes())
            .map_err(transport)?;
    }
    OutgoingBody::finish(outgoing, None).map_err(transport)?;
    pending.subscribe().block();
    let response = pending
        .get()
        .ok_or_else(|| transport("response not ready"))?
        .map_err(transport)?
        .map_err(transport)?;
    let status = response.status();
    let retry_after = response
        .headers()
        .get("retry-after")
        .first()
        .map(|v| String::from_utf8(v.clone()))
        .transpose()
        .map_err(transport)?;
    let incoming = response.consume().map_err(transport)?;
    let mut bytes = Vec::new();
    {
        let stream = incoming.stream().map_err(transport)?;
        loop {
            match stream.blocking_read(16_384) {
                Ok(data) => {
                    if bytes.len() + data.len() > MAX_BODY {
                        return Err(FetchError::Upstream("HTTP response exceeds 16 MiB".into()));
                    }
                    bytes.extend(data);
                }
                Err(StreamError::Closed) => break,
                Err(error) => return Err(transport(error)),
            }
        }
    }
    let trailers = IncomingBody::finish(incoming);
    trailers.subscribe().block();
    trailers
        .get()
        .ok_or_else(|| transport("trailers not ready"))?
        .map_err(transport)?
        .map_err(transport)?;
    Ok(Response {
        status,
        body: bytes,
        retry_after,
    })
}

#[cfg(not(target_os = "wasi"))]
async fn request(_url: &str, _token: &str, _body: Option<&str>) -> Result<Response, FetchError> {
    Err(FetchError::Config("HTTP requires a WASI runtime".into()))
}

async fn sleep(milliseconds: u64) {
    #[cfg(all(target_os = "wasi", target_env = "p3"))]
    wasip3::clocks::monotonic_clock::wait_for(milliseconds * 1_000_000).await;
    #[cfg(all(target_os = "wasi", not(target_env = "p3")))]
    wasi::clocks::monotonic_clock::subscribe_duration(milliseconds * 1_000_000).block();
    #[cfg(not(target_os = "wasi"))]
    let _ = milliseconds;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn honors_bounded_retry_after() {
        assert_eq!(retry_delay(Some("60"), 0).unwrap(), 60_000);
        assert_eq!(retry_delay(None, 2).unwrap(), 4000);
        assert!(matches!(
            retry_delay(Some("121"), 0),
            Err(FetchError::RateLimited(_))
        ));
        assert!(retry_delay(Some("invalid"), 0).is_err());
    }
}
