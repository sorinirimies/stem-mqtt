# syntax=docker/dockerfile:1

# ── Builder ────────────────────────────────────────────────────────────────
# Multi-stage so `docker build`/`docker compose up` work standalone — no
# host-side `cargo build` prerequisite (unlike the single-stage version
# this replaced, which only copied an already-built binary).
FROM rust:1-slim-bookworm AS builder
WORKDIR /build

RUN apt-get update && apt-get install -y --no-install-recommends pkg-config \
    && rm -rf /var/lib/apt/lists/*

# Copy the whole workspace: mqtt-broker depends on mqtt-client via a path
# dependency, and Cargo needs every workspace member's manifest present to
# resolve the graph even when only building one member with `-p`.
COPY Cargo.toml Cargo.lock ./
COPY crates/mqtt-client ./crates/mqtt-client
COPY crates/mqtt-broker ./crates/mqtt-broker
COPY crates/mqtt-client-node/Cargo.toml ./crates/mqtt-client-node/Cargo.toml
COPY crates/mqtt-client-node/build.rs ./crates/mqtt-client-node/build.rs
COPY crates/mqtt-client-node/src ./crates/mqtt-client-node/src

RUN cargo build --release -p mqtt-broker --bin mqtt-broker

# ── Runtime ──────────────────────────────────────────────────────────────────
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=builder /build/target/release/mqtt-broker /usr/local/bin/mqtt-broker

# 1883 = raw MQTT-over-TCP, 8083 = MQTT-over-WebSocket (conventional MQTT
# WS port; only listened on if --ws-port is passed).
EXPOSE 1883 8083

ENTRYPOINT ["mqtt-broker"]
CMD ["--bind", "0.0.0.0", "--port", "1883"]
