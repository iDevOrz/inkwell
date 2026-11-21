FROM rust:1-slim-bookworm AS builder

WORKDIR /app

RUN apt-get update \
    && apt-get install -y --no-install-recommends build-essential pkg-config \
    && rm -rf /var/lib/apt/lists/*

COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY migrations ./migrations
COPY templates ./templates
COPY static ./static

RUN cargo build --release --locked

FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --create-home app \
    && mkdir -p /app/data \
    && chown -R app:app /app

WORKDIR /app
COPY --from=builder /app/target/release/typecho-rs /usr/local/bin/typecho-rs

USER app
ENV DATA_DIR=/app/data \
    BIND_ADDR=0.0.0.0:3000 \
    RUST_LOG=typecho_rs=info,tower_http=info

VOLUME ["/app/data"]
EXPOSE 3000

CMD ["typecho-rs", "serve"]
