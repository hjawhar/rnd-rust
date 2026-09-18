# syntax=docker/dockerfile:1.7
#
# Multi-stage build for `irc-server`.
# Dependency compilation is cached separately via cargo-chef so incremental
# rebuilds touch only changed workspace crates. Final layer is distroless
# nonroot — no shell, no package manager, UID 65532.
#
# Build:
#   docker build -f ops/docker/irc-server.Dockerfile -t irc-server .
# Run:
#   docker run --rm -p 6667:6667 -p 6697:6697 -p 9772:9772 \
#     -v $(pwd)/server-config:/etc/app:ro -v irc-server-data:/var/lib/app \
#     irc-server

ARG RUST_VERSION=1.90

FROM rust:${RUST_VERSION}-slim-bookworm AS chef
RUN cargo install cargo-chef --locked --version ^0.1
WORKDIR /src

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
ARG BIN=irc-server
COPY --from=planner /src/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json
COPY . .
RUN cargo build --release --locked --bin ${BIN}

FROM gcr.io/distroless/cc-debian12:nonroot
ARG BIN=irc-server
COPY --from=builder /src/target/release/${BIN} /usr/local/bin/app
USER nonroot
EXPOSE 6667 6697 9772
VOLUME ["/var/lib/app", "/etc/app"]
ENTRYPOINT ["/usr/local/bin/app"]
CMD ["--config", "/etc/app/config.toml"]
