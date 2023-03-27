FROM rust:1.68.1 AS builder
RUN apt-get update && apt-get -y upgrade
COPY . .
RUN cargo build --release

FROM debian:buster-slim
COPY --from=builder ./target/release/blob-indexer ./target/release/blob-indexer
CMD ["/target/release/blob-indexer"]