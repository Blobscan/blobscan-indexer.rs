# Blobscan indexer <a href="#"><img align="right" src=".github/assets/blobi.jpeg" height="80px" /></a>

The indexer for the [Blobscan](https://github.com/Blobscan/blobscan) explorer implemented in Rust.

## How it works?

The indexer crawls the blockchain fetching information from both the Execution and Beacon clients. The data is processed and stored in a MongoDB database.

## How to run locally?

1. Git clone this repo.

```bash
git clone https://github.com/Blobscan/blobscan-indexer.rs.git
```

2. Go to directory.

```bash
cd blobscan-indexer.rs
```

3. Set the environment variables.

4. Run the indexer.

```bash
cargo run
```

5. Optionally run the indexer using the docker image:

```bash
docker run --rm -e DB_CONNECTION_URI=mongodb://blobscan:secret@127.0.0.1:27017 -e DB_NAME=blobscan blossomlabs/blobscan-indexer
```

## Environment variables

Create a `.env` file with environment variables. You can use the `.env.example` file as a reference.

Below you can find a list of all the required variables:

| Env variable        | Description                                                    |
| ------------------- | -------------------------------------------------------------- |
| `DB_CONNECTION_URI` | The connection URI to connect to a deployed database instance. |
| `DB_NAME`           | The database's name.                                           |

And here a list of optional variables:

| Env variable         | Description                                                                                     | Default value           |
| -------------------- | ----------------------------------------------------------------------------------------------- | ----------------------- |
| `BEACON_NODE_RPC`    | A beacon chain RPC node's endpoint.                                                             | `http://localhost:3500` |
| `EXECUTION_NODE_URL` | An execution RPC node's endpoint.                                                               | `http://localhost:8545` |
| `LOGGER`             | The logger's name to be used. See log4rs [config file](log4rs.yml) to check the available ones. | `default`               |

# About Blossom Labs

![blossom labs](https://blossom.software/img/logo.svg)

Blobscan is being developed by [Blossom Labs](https://blossom.software/), a developer team specialized in building blockchain-based infrastructure for online communities.
