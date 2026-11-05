# syntax=docker/dockerfile:1
FROM --platform=$BUILDPLATFORM rust:1.98.1-bookworm@sha256:93ce27a88655056a51dbdd8f5f2d7ddc071c7b0070fb288a37b5a285fc83971e AS build
WORKDIR /build/SaveEyeEdgeListenerApp
RUN rustup target add wasm32-wasip2
COPY SaveEyeEdgeListenerApp/Cargo.toml SaveEyeEdgeListenerApp/Cargo.lock ./
COPY SaveEyeEdgeListenerApp/src/ src/
COPY saveeye-telemetry/ /build/saveeye-telemetry/
RUN cargo build --locked --release --target wasm32-wasip2 -j 2

FROM ghcr.io/spinframework/spin:v4.2.6@sha256:bcb6bb11aef4fc32aadd59bac499f8db927950cfad41050f7005b90a1d73fc55
ENV HOME=/home/spin
RUN useradd --create-home --uid 10001 spin
USER spin
RUN spin plugins install --url https://github.com/spinframework/spin-trigger-plugins/releases/download/mqtt-v0.9.6/trigger-mqtt.json --yes
WORKDIR /home/spin/app
COPY --chown=10001:10001 SaveEyeEdgeListenerApp/spin.toml ./
COPY --from=build --chown=10001:10001 /build/SaveEyeEdgeListenerApp/target/wasm32-wasip2/release/save_eye_edge_listener_app.wasm target/wasm32-wasip2/release/
CMD ["up", "--from", "/home/spin/app/spin.toml", "--state-dir="]
