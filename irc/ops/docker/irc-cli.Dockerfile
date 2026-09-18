# syntax=docker/dockerfile:1.7
# See irc-server.Dockerfile for commentary. This file builds the headless
# TUI client used for smoke tests and ops scripting.

ARG RUST_VERSION=1.90

FROM rust:${RUST_VERSION}-slim-bookworm AS chef
RUN cargo install cargo-chef --locked --version ^0.1
WORKDIR /src

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
ARG BIN=irc-cli
COPY --from=planner /src/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json
COPY . .
RUN cargo build --release --locked --bin ${BIN}

FROM gcr.io/distroless/cc-debian12:nonroot
ARG BIN=irc-cli
COPY --from=builder /src/target/release/${BIN} /usr/local/bin/app
USER nonroot
ENTRYPOINT ["/usr/local/bin/app"]
