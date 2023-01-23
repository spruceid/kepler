![kepler header](/docs/keplerheader.png)

[![](https://img.shields.io/badge/License-Apache--2.0-green)](https://github.com/spruceid/kepler/blob/main/LICENSE) [![](https://img.shields.io/twitter/follow/spruceid?label=Follow&style=social)](https://twitter.com/spruceid)

# Kepler

Kepler is self-sovereign storage. It is architected as a decentralized storage system that uses DIDs and Authorization Capabilities to define Orbits, where your data lives and who has access. Any DID controller (e.g. people, applications, DAOs) can administer their own Kepler Orbit.

## Quickstart

To run Kepler locally you will need the latest version of [rust](https://rustup.rs).

You will need to create a directory for Kepler to store data in:
```bash
mkdir kepler
```

Within this directory, create two more directories `blocks` and `indexes`:
```bash
mkdir kepler/blocks
mkdir kepler/indexes
```

You will then need to set the environment variables to point to those directories:
```bash
export KEPLER_STORAGE_BLOCKS_PATH="kepler/blocks"
export KEPLER_STORAGE_INDEXES_PATH="kepler/indexes"
```

Finally you can run Kepler using `cargo`:
```bash
cargo build
cargo run
```


## Configuration

Kepler instances are configured by the [kepler.toml](kepler.toml) configuration file, or via environment variables. You can either modify them in this file, or specify them through environment variable using the prefix `KEPLER_`.

The following common options are available:

| Option               | env var                     | description                                                    |
|:---------------------|:----------------------------|:---------------------------------------------------------------|
| log_level            | KEPLER_LOG_LEVEL            | Set the level of logging output, options are "normal", "debug" |
| address              | KEPLER_ADDRESS              | Set the listening address of the kepler instance               |
| port                 | KEPLER_PORT                 | Set the listening TCP port for the kepler instance             |
| storage.blocks.type  | KEPLER_STORAGE_BLOCKS_TYPE  | Set the mode of block storage, options are "Local" and "S3"    |
| storage.indexes.type | KEPLER_STORAGE_INDEXES_TYPE | Set the type of the index store, options are "Local" and "DynamoDB" |
| orbits.allowlist     | KEPLER_ORBITS_ALLOWLIST     | Set the URL of an allowlist service for gating the creation of Orbit Peers                                                               |

### Storage Config

Storage can be configured for both Blocks and Indexes, depending on the `type` for each.

#### Local Storage

When `storage.blocks.type` and `storage.indexes.type` are `Local`, the local filesystem will be used for application storage. The following config options will become available:

| Option               | env var                     | description                                                    |
|:---------------------|:----------------------------|:---------------------------------------------------------------|
| storage.blocks.path  | KEPLER_STORAGE_BLOCKS_PATH  | Set the path of the block storage                              |
| storage.indexes.path | KEPLER_STORAGE_INDEXES_PATH | Set the path of the index store                                |

#### AWS Storage

When `storage.blocks.type` is `S3` and `storage.indexes.type` is `DynamoDB`, the instance will use the S3 and DynamoDB AWS services for application storage. The following config options will become available:

| Option               | env var                     | description                                                    |
|:---------------------|:----------------------------|:---------------------------------------------------------------|
| storage.blocks.type  | KEPLER_STORAGE_BLOCKS_TYPE  | Set the mode of block storage, options are "Local" and "S3"    |
| storage.blocks.bucket  | KEPLER_STORAGE_BLOCKS_BUCKET  | Set the name of the S3 bucket    |
| storage.blocks.endpoint  | KEPLER_STORAGE_BLOCKS_ENDPOINT  | Set the URL of the S3 store    |
| storage.blocks.dynamodb_table  | KEPLER_STORAGE_BLOCKS_DYNAMODB_TABLE  | Set the name of the dynamodb table |
| storage.blocks.dynamodb_endpoint  | KEPLER_STORAGE_BLOCKS_DYNAMODB_ENDPOINT  | Set the URL of the dynamodb service  |
| storage.indexes.path | KEPLER_STORAGE_INDEXES_PATH | Set the path of the index store                                |

Additionally, the following environment variables must be present: `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY` and `AWS_DEFAULT_REGION`.

## Running

Kepler instances can be started via command line, e.g.:

``` sh
KEPLER_PORT=8001 kepler
```

If the Kepler instance is not able to find or establish a connection to the configured storage, the instance will terminate.

## Usage

Kepler is most easily used via the [Kepler SDK](https://github.com/spruceid/kepler-sdk). See the example DApps and tutorials for detailed information.
