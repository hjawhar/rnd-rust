# Multi-stage build for all Rust services.
# Produces three binaries: bambu-gateway, bambu-api, bambu-bridge.

# --- Build stage ---
FROM rust:1.90-bookworm AS builder

# Install libpq-dev for diesel postgres support
RUN apt-get update && apt-get install -y libpq-dev protobuf-compiler && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy manifests first for dependency caching
COPY Cargo.toml Cargo.lock ./
COPY crates/shared/Cargo.toml crates/shared/
COPY crates/gateway/Cargo.toml crates/gateway/
COPY crates/api/Cargo.toml crates/api/
COPY crates/bridge/Cargo.toml crates/bridge/

# Create dummy source files so cargo can resolve deps
RUN mkdir -p crates/shared/src && echo "pub fn _dummy() {}" > crates/shared/src/lib.rs \
    && mkdir -p crates/gateway/src && echo "fn main() {}" > crates/gateway/src/main.rs \
    && mkdir -p crates/api/src && echo "fn main() {}" > crates/api/src/main.rs \
    && mkdir -p crates/bridge/src && echo "fn main() {}" > crates/bridge/src/main.rs

# Copy proto files needed by build.rs
COPY proto/ proto/

# Create a minimal build.rs so shared compiles during dep caching
COPY crates/shared/build.rs crates/shared/

# Build dependencies only (cached layer)
RUN cargo build --release --workspace 2>/dev/null || true

# Now copy actual source code
COPY crates/ crates/

# Touch source files to invalidate the dummy builds
RUN touch crates/shared/src/lib.rs \
    crates/gateway/src/main.rs \
    crates/api/src/main.rs \
    crates/bridge/src/main.rs

# Build all binaries
RUN cargo build --release --workspace

# --- Runtime stage ---
FROM debian:bookworm-slim AS runtime

RUN apt-get update && apt-get install -y libpq5 ca-certificates curl && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/bambu-gateway /usr/local/bin/
COPY --from=builder /app/target/release/bambu-api /usr/local/bin/
COPY --from=builder /app/target/release/bambu-bridge /usr/local/bin/

# Non-root user
RUN useradd -r -s /bin/false bambu
USER bambu
