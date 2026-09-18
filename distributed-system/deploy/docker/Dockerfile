# === Builder ===
FROM rust:1.90-bookworm AS builder

ARG CRATE_NAME

WORKDIR /app

# Copy workspace manifests first for layer caching
COPY Cargo.toml Cargo.lock ./
COPY crates/common/Cargo.toml crates/common/Cargo.toml
COPY crates/gateway/Cargo.toml crates/gateway/Cargo.toml
COPY crates/processor/Cargo.toml crates/processor/Cargo.toml
COPY crates/enricher/Cargo.toml crates/enricher/Cargo.toml
COPY crates/aggregator/Cargo.toml crates/aggregator/Cargo.toml

# Create dummy source files to build dependencies first (layer cache optimization)
RUN mkdir -p crates/common/src && echo "pub fn dummy() {}" > crates/common/src/lib.rs \
    && mkdir -p crates/gateway/src && echo "fn main() {}" > crates/gateway/src/main.rs \
    && mkdir -p crates/processor/src && echo "fn main() {}" > crates/processor/src/main.rs \
    && mkdir -p crates/enricher/src && echo "fn main() {}" > crates/enricher/src/main.rs \
    && mkdir -p crates/aggregator/src && echo "fn main() {}" > crates/aggregator/src/main.rs

# Build dependencies only (cached unless Cargo.toml changes)
RUN cargo build --release -p ${CRATE_NAME} || true

# Now copy real source code
COPY crates/ crates/

# Touch source files to invalidate the build cache for actual source
RUN find crates -name "*.rs" -exec touch {} +

# Build the actual binary
RUN cargo build --release -p ${CRATE_NAME}

# === Runtime ===
FROM debian:bookworm-slim

ARG CRATE_NAME

# Install runtime dependencies (libpq for diesel/postgres if aggregator)
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates libpq5 \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/${CRATE_NAME} /usr/local/bin/service

# Copy migrations for aggregator (no-op for other services)
COPY crates/aggregator/migrations /app/migrations

EXPOSE 3000 3001

CMD ["service"]
