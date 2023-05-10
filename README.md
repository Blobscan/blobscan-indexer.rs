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
docker run -e BLOBSCAN_API_ENDPOINT=http://blobscan-api:3001 -e BEACON_NODE_RPC=http://beacon:3500 -e EXECUTION_NODE_URL=http://execution:8545 --rm blossomlabs/blobscan-indexer:master
docker run --env-file=.env --rm blossomlabs/blobscan-indexer:master
```

For more information, check out [blossomlabs/blobscan-indexer](https://hub.docker.com/repository/docker/blossomlabs/blobscan-indexer/general) on Docker Hub.

## Environment variables

Below you can find a list of supported variables:

| Name                    | Required | Description                                                                            | Default value           |
| ----------------------- | -------- | -------------------------------------------------------------------------------------- | ----------------------- |
| `SECRET_KEY`            | **Yes**  | Shared secret key Blobscan API JWT authentication.                                     |                         |
| `BLOBSCAN_API_ENDPOINT` | No       | Endpoint for the Blobscan API.                                                         | `http://localhost:3001` |
| `BEACON_NODE_RPC`       | No       | A consensus client RPC endpoint.                                                       | `http://localhost:3500` |
| `EXECUTION_NODE_URL`    | No       | An execution client RPC endpoint.                                                      | `http://localhost:8545` |

# About Blossom Labs

![blossom labs](https://blossom.software/img/logo.svg)

Blobscan is being developed by [Blossom Labs](https://blossom.software/), a developer team specialized in building blockchain-based infrastructure for online communities.

[Join us on Discord!](https://discordapp.com/invite/fmqrqhkjHY/)
