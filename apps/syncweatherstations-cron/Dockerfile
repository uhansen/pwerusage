FROM ghcr.io/spinframework/spin:v4.2.3

RUN spin plugins install trigger-cron --yes

WORKDIR /app
COPY apps/syncweatherstations-cron/ /app/

CMD ["up", "--from", "/app/spin.compose.toml", "--experimental-wasi-otel", "--follow", "syncweatherstations-cron"]
