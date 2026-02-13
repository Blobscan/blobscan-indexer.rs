FROM lukemathwalker/cargo-chef:latest-rust-1 AS chef
WORKDIR app

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json
COPY . .
RUN cargo build --release --bin blob-indexer

FROM debian:trixie-slim
RUN apt-get update -y && apt-get install -y ca-certificates
WORKDIR /app
COPY --from=builder /app/target/release/blob-indexer .
ENTRYPOINT ["/app/blob-indexer"]
