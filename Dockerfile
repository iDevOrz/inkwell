FROM rust:1-slim-bookworm@sha256:7c7d4eb4d05b7781187a327809466cfddf29753f6defdc3d19a0e291951b7a37 AS builder

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

FROM debian:bookworm-slim@sha256:e67eeead736ff7bbe258ef088bff683635c561b1fcfefad0806c1df4ab49ce0e

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --create-home app \
    && mkdir -p /app/data \
    && chown -R app:app /app

WORKDIR /app
COPY --from=builder /app/target/release/inkwell /usr/local/bin/inkwell

USER app
ENV DATA_DIR=/app/data \
    BIND_ADDR=0.0.0.0:3000 \
    RUST_LOG=inkwell=info,tower_http=info

VOLUME ["/app/data"]
EXPOSE 3000

# Probes GET /healthz (database reachable) without needing curl in the image.
HEALTHCHECK --interval=30s --timeout=5s --start-period=30s --start-interval=2s --retries=3 \
    CMD ["inkwell", "healthcheck"]

CMD ["inkwell", "serve"]
