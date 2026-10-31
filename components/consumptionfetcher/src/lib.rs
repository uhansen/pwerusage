wit_bindgen::generate!({
    path: "wit",
    world: "consumption-fetcher",
});

pub use pwrusage::consumption_fetcher::types::*;

mod fetch;
mod http;
pub mod model;

struct Component;

impl exports::pwrusage::consumption_fetcher::fetcher::Guest for Component {
    async fn fetch(
        address: Address,
        date_from: Option<String>,
        date_to: Option<String>,
    ) -> Result<FetchSummary, FetchError> {
        fetch::run(address, date_from, date_to).await
    }
}

#[cfg(target_family = "wasm")]
export!(Component);
