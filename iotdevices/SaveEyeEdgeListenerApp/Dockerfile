# syntax=docker/dockerfile:1
FROM --platform=$BUILDPLATFORM rust:1.98.1-bookworm@sha256:93ce27a88655056a51dbdd8f5f2d7ddc071c7b0070fb288a37b5a285fc83971e AS build
WORKDIR /build/iotdevices/SaveEyeEdgeListenerApp
RUN rustup target add wasm32-wasip2
RUN cargo install wac-cli --version 0.10.1 --locked -j 2 && \
    cargo install wasm-tools --version 1.261.0 --locked --no-default-features --features component -j 2
COPY iotdevices/SaveEyeEdgeListenerApp/Cargo.toml iotdevices/SaveEyeEdgeListenerApp/Cargo.lock ./
COPY iotdevices/SaveEyeEdgeListenerApp/src/ src/
COPY iotdevices/SaveEyeEdgeListenerApp/build.sh ./
COPY components/saveeye-telemetry/ /build/components/saveeye-telemetry/
RUN bash build.sh

FROM ghcr.io/spinframework/spin:v4.2.6@sha256:bcb6bb11aef4fc32aadd59bac499f8db927950cfad41050f7005b90a1d73fc55
ENV HOME=/home/spin
RUN useradd --create-home --uid 10001 spin
USER spin
RUN spin plugins install --url https://github.com/spinframework/spin-trigger-plugins/releases/download/mqtt-v0.9.6/trigger-mqtt.json --yes
WORKDIR /home/spin/app
COPY --chown=10001:10001 iotdevices/SaveEyeEdgeListenerApp/spin.toml ./
COPY --from=build --chown=10001:10001 /build/iotdevices/SaveEyeEdgeListenerApp/composed.wasm ./
CMD ["up", "--from", "/home/spin/app/spin.toml", "--state-dir="]
