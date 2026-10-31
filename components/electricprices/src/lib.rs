wit_bindgen::generate!({
    path: "wit",
    world: "electric-prices",
});

pub use pwrusage::electric_prices::types::*;

mod fetch;
mod http;
pub mod model;

struct Component;

impl exports::pwrusage::electric_prices::fetcher::Guest for Component {
    async fn fetch(
        provider: Provider,
        position: Position,
        history_from: Option<String>,
        history_to: Option<String>,
    ) -> Result<FetchSummary, FetchError> {
        fetch::run(provider, position, history_from, history_to).await
    }
}

#[cfg(target_family = "wasm")]
export!(Component);
