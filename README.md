# Blobscan indexer <a href="#"><img align="right" src=".github/assets/blobi.jpeg" height="80px" /></a>

The indexer for the [Blobscan](https://github.com/Blobscan/blobscan) explorer implemented in Rust.

## How it works?

The indexer crawls the beaconchain fetching information from both the Execution and Consensus clients. The data is collected and sent to the Blobscan API that will store it in a PostgreSQL database.

## Compile and run locally

1. Install dependencies

```
sudo apt install libssl-dev
```

2. Git clone this repository.

```bash
git clone https://github.com/Blobscan/blobscan-indexer.rs.git
cd blobscan-indexer.rs
```

3. Set the environment variables.

**blobscan-indexer** interacts with other services (such as the execution and consensus clients). In a system where the defaults are not correct, they can be configured
by using environment variables or by creating a `.env` file. You can use the `.env.example` file as a reference.

```bash
echo "SECRET_KEY=$(openssl rand -base64 32)" > .env
```

For more information about available variables check out [the table below](#environment-variables).

4. Run the indexer.

```bash
cargo run
```

5. Build a release

```bash
cargo build -r
```

## Docker images

For convenience, we also provide docker images for blobscan-indexer.

Running with defaults

```bash
docker run --rm blossomlabs/blobscan-indexer:master
```

Using environment variables

```bash
docker run --rm \
  -e BLOBSCAN_API_ENDPOINT=http://blobscan-api:3001 \
  -e BEACON_NODE_URL=http://beacon:3500 \
  -e EXECUTION_NODE_RPC=http://execution:8545 \
  blossomlabs/blobscan-indexer:master
```

Or directly using the .env file

```
docker run --env-file=.env --rm blossomlabs/blobscan-indexer:master
```

For more information, check out [Docker Hub](https://hub.docker.com/r/blossomlabs/blobscan-indexer).

## Environment variables

Below you can find a list of supported variables:

| Name                    | Required | Description                                        | Default value           |
| ----------------------- | -------- | -------------------------------------------------- | ----------------------- |
| `SECRET_KEY`            | **Yes**  | Shared secret key Blobscan API JWT authentication. |                         |
| `BLOBSCAN_API_ENDPOINT` | No       | Endpoint for the Blobscan API.                     | `http://localhost:3001` |
| `BEACON_NODE_URL`       | No       | A consensus client RPC endpoint.                   | `http://localhost:3500` |
| `EXECUTION_NODE_URL`    | No       | An execution client RPC endpoint.                  | `http://localhost:8545` |
| `SENTRY_DSN`            | No       | Sentry client key.                                 |                         |

## Command-Line Arguments

The indexer supports the following command-line arguments for configuring the indexing process:

- `-f, --from-slot <FROM_SLOT>`: It allows you to specify the starting slot for indexing ignoring the default behavior, which is starting from the latest slot stored in the database.

- `-n, --num-threads <NUM_THREADS>`: It allows you to specify the number of threads that will be utilized to parallelize the indexing process. If the argument is not provided, the number of cores of the machine will be used.
- `-s, --slots-per-save <SLOTS_PER_SAVE>`: It allows you to specify the number of slots to be processed before saving the latest slot in the database.

### Example usage

```sh
cargo run -- -f 1000 -n 10
```

## A note on tracing

The indexer uses the [`EnvFilter`](https://docs.rs/tracing-subscriber/latest/tracing_subscriber/filter/struct.EnvFilter.html) and [`Bunyan`](https://docs.rs/tracing-bunyan-formatter/0.1.6/tracing_bunyan_formatter/struct.BunyanFormattingLayer.html) tracing layers to provide more customizable and legible events by using the bunyan format.

To display the formatted logs you'll need to have the bunyan CLI [installed](https://github.com/LukeMathWalker/bunyan#how-to-install) and pipe the indexer's output to the bunyan cli as shown below:

```sh
cargo run -q | bunyan
```

To filter spans and events through the `EnvFilter` layer you can use the default env variable `RUST_LOG` to define the directives to be used.

```sh
RUST_LOG=blob_indexer[span{field=value}]=level cargo run
```

# About Blossom Labs

![blossom labs](https://blossom.software/img/logo.svg)

Blobscan is being developed by [Blossom Labs](https://blossom.software/), a developer team specialized in building blockchain-based infrastructure for online communities.

[Join us on Discord!](https://discordapp.com/invite/fmqrqhkjHY/)
