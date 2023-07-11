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

Within this directory, create one more directories `blocks` and a database file `caps.db`:
```bash
mkdir kepler/blocks
touch kepler/caps.db
```

You will then need to set the environment variables to point to those directories:
```bash
export KEPLER_STORAGE_BLOCKS_PATH="kepler/blocks"
export KEPLER_STORAGE_DATABASE="kepler/caps.db"
```

Finally you can run Kepler using `cargo`:
```bash
cargo build
cargo run
```


## Configuration

Kepler instances are configured by the [kepler.toml](kepler.toml) configuration file, or via environment variables. You can either modify them in this file, or specify them through environment variable using the prefix `KEPLER_`.

The following common options are available:

| Option              | env var                    | description                                                                |
|:--------------------|:---------------------------|:---------------------------------------------------------------------------|
| log_level           | KEPLER_LOG_LEVEL           | Set the level of logging output, options are "normal", "debug"             |
| address             | KEPLER_ADDRESS             | Set the listening address of the kepler instance                           |
| port                | KEPLER_PORT                | Set the listening TCP port for the kepler instance                         |
| storage.blocks.type | KEPLER_STORAGE_BLOCKS_TYPE | Set the mode of block storage, options are "Local" and "S3"                |
| storage.limit        | KEPLER_STORAGE_LIMIT        | Set a maximum limit on storage available to Orbits hosted on this instance. Limits are written as strings, e.g. `10 MiB`, `100 GiB`                                                                           |
| storage.database    | KEPLER_STORAGE_DATABASE    | Set the location of the SQL database                                       |
| storage.staging     | KEPLER_STORAGE_STAGING     | Set the mode of content staging, options are "Memory" and "FileSystem"     |
| keys.type           | KEPLER_KEYS_TYPE           | Set the type of host key store, options are "Static"                       |
| orbits.allowlist    | KEPLER_ORBITS_ALLOWLIST    | Set the URL of an allowlist service for gating the creation of Orbit Peers |

### Database Config

The SQL database can be configured with `storage.database` or the `KEPLER_STORAGE_DATABASE` environment variable. It supports Sqlite, MySQL and PostgresSQL. For example:

| Type     | Example                                       | Description                                                                         |
|:---------|:----------------------------------------------|:------------------------------------------------------------------------------------|
| Sqlite   | "sqlite:./kepler/caps.db"                     | Set Kepler to use a local Sqlite file at the relative path `./kepler/caps.db`       |
| MySQL    | "mysql://root:root@localhost:3306/example"    | Use the MySQL instance deployed at `localhost:3306`, with database name `example`   |
| Postgres | "postgres://root:root@localhost:5432/example" | Use the Postgres instance deployed at `localhost:5432` with database name `example` |

This will default to an in-memory Sqlite database (i.e. `sqlite::memory:`).

#### Migrations

Kepler will automatically apply the relevant migrations to your chosen SQL database. Use caution if you are sharing this database with another application.

### Staging Config

Kepler will temporarily stage files it recieves before writing them. It can do this in memory or in temporary files. This can be configured by setting `storage.staging` to `Memory` or `FileSystem`. Default is `Memory`.

### Storage Config

Storage can be configured for Blocks depending on it's `type`.

#### Local Storage

When `storage.blocks.type` is `Local`, the local filesystem will be used for application content storage. The following config option will become available:

| Option               | env var                     | description                                                    |
|:---------------------|:----------------------------|:---------------------------------------------------------------|
| storage.blocks.path  | KEPLER_STORAGE_BLOCKS_PATH  | Set the path of the block storage                              |

#### AWS Storage

When `storage.blocks.type` is `S3` the instance will use the S3 AWS service for application storage. The following config options will become available:

| Option               | env var                     | description                                                    |
|:---------------------|:----------------------------|:---------------------------------------------------------------|
| storage.blocks.type  | KEPLER_STORAGE_BLOCKS_TYPE  | Set the mode of block storage, options are "Local" and "S3"    |
| storage.blocks.bucket  | KEPLER_STORAGE_BLOCKS_BUCKET  | Set the name of the S3 bucket    |
| storage.blocks.endpoint  | KEPLER_STORAGE_BLOCKS_ENDPOINT  | Set the URL of the S3 store    |

Additionally, the following environment variables must be present: `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY` and `AWS_DEFAULT_REGION`.

### Keys Config

Kepler hosts require key pairs to provide replication. The `keys` config fields specify how a Kepler instance generates and stores these key pairs.

#### Static Secret Derivation

When `keys.type` is `Static` the instance will use an array of bytes as a static secret from which it will derive key pairs on a per-Orbit basis. The following config options will be available:

| Option      | env var            | description                                                                  |
|:------------|:-------------------|:-----------------------------------------------------------------------------|
| keys.secret | KEPLER_KEYS_SECRET | Unpadded base64Url-encoded byte string from which key pairs will be derived. |

The secret MUST contain at least 32 bytes of entropy (either randomly generated or derived in a cryptographically secure way). It is STRONGLY RECOMMENDED that the secret be given via environment variables and NOT in the `kepler.toml` config file. Additionally it is STRONGLY RECOMMENDED that the secret be backed up in a secure place if used in production. Loss of the secret will result in total loss of function for the Kepler instance.

## Running

Kepler instances can be started via command line, e.g.:

``` sh
KEPLER_PORT=8001 kepler
```

If the Kepler instance is not able to find or establish a connection to the configured storage, the instance will terminate.

## Usage

Kepler is most easily used via the [Kepler SDK](https://github.com/spruceid/kepler-sdk). See the example DApps and tutorials for detailed information.
