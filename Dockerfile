# syntax=docker/dockerfile:1
# Bookworm build + runtime for the mina-verify binaries.

FROM rust:bookworm AS builder
# rust:bookworm ships rustup; rust-toolchain.toml pins the exact toolchain (1.94.1).
RUN apt-get update && apt-get install -y --no-install-recommends \
        build-essential pkg-config libssl-dev git ca-certificates \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY . .
RUN cargo build --release --locked
RUN strip \
        target/release/mina-verify \
        target/release/mina-verify-capture \
        target/release/mina-verify-monitor

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates libssl3 \
    && rm -rf /var/lib/apt/lists/*
COPY --from=builder /src/target/release/mina-verify          /usr/local/bin/mina-verify
COPY --from=builder /src/target/release/mina-verify-capture  /usr/local/bin/mina-verify-capture
COPY --from=builder /src/target/release/mina-verify-monitor  /usr/local/bin/mina-verify-monitor
# default: the live verify-before-ingest monitor (MINA_NETWORK env selects the network)
ENTRYPOINT ["mina-verify-monitor"]
