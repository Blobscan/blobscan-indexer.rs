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

FROM debian:bullseye-slim
RUN apt-get update && apt-get -y install libssl1.1
WORKDIR app
ENV DB_CONNECTION_URI=mongodb://blobscan:secret@127.0.0.1:27017
ENV DB_NAME=blobscan_dev
RUN echo "DB_CONNECTION_URI=$DB_CONNECTION_URI" > .env
RUN echo "DB_NAME=$DB_NAME" >> .env
COPY --from=builder /app/target/release/blob-indexer .
ENTRYPOINT ["/app/blob-indexer"]
