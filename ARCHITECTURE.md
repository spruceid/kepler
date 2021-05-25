# Kepler Architecture

Authors: Wayne Chang <wayne@spruceid.com>

Date: May 24th, 2021

## Major Subsystems
Kepler has these major subsystems:
- Orbit Manifest: Provides Root Authority, Discoverability, and Base Policy
- Access Control: Modular Key-Based Authentication, Capabilities-Based Permissioning
- Hosts: Logical Units Serving Requests
- Geometry: Mappings across Orbits inspired by [FreeBSD's GEOM](https://people.freebsd.org/~phk/Geom/).

### Orbit Manifest
Orbits are data overlays that are managed by keypairs. It is defined by an
Orbit Manifest.

The Orbit Manifest is a digital document that describes all the important
aspects about the the Orbit, namely:
- The latest stable content state (e.g., via Merkle root hash).
- The base access control list.
- The list of hosts from which clients list, fetch, and update content.
- The revocation strategy and validity status for capabilities.
- The data consistency strategies for content and policy.
- The supported authentication methods.

The Orbit Manifest can live:
- A smart contract on a censorship-resistant blockchain (e.g,. Tezos, Ethereum, Solana, etc.)
- A smart contract on a private blockchain (e.g., Fabric, Corda)
- An updatable document with high availability, such as Ceramic documents or Textile instances.
- Permissioned DHT-like with update capabilities such as ipfs-log or [DHT Mutable Items](http://bittorrent.org/beps/bep_0046.html).
- A centralized storage provider: website, S3, GitHub pages.
- Any of the options above but also encrypted.

Ultimately, the storage medium for the Orbit Manifest will depend on
requirements for discoverability, performance, permissioning, and resilience.
For example, many smart contract languges allow Orbit Manifest permissioning to
be enforced at the blockchain VM level.

#### Orbit Identifiers and Orbit Manifest Resolution
Orbit Identifiers allow users to resolve an Orbit Manifest. They are URIs
([RFC3986](https://datatracker.ietf.org/doc/html/rfc3986#section-3.1)) with
parameters that differ depending on the Orbit Method. Orbit Identifiers are
the Orbit Method type identifier followed by query parameters and fragments
defined in the Orbit Method type specification.

For example, Orbit Identifiers from the Tezos Orbit Method:
```
tz?account=tz1TUh4tk6xRGsrwstKFw88sapBs8iLB3LrP&host=kepler.tzprofiles.com&nonce=jVUYDuxJ
tz?account=KT1XgKpd8KwyBUyE1Sfn8uXMr6qidRXJeM4B
```

To go from an Orbit Identifier to an Kepler URI, the identifier is first
optionally hashed into a
[multiformats representation](https://multiformats.io/). This is desirable to
add privacy (especially if there is a nonce-like field) or keep a constant
Kepler URI size. It is then prefixed with `kepler://`.

Examples of Kepler URIs:
```
kepler://tz?account=tz1TUh4tk6xRGsrwstKFw88sapBs8iLB3LrP&host=kepler.tzprofiles.com&nonce=jVUYDuxJ
kepler://F9bdb90a11d5b6ffc6f07f0a2f90563f0d38f1751585f6bb656c26e3d83b411ae
kepler://tz?account=KT1XgKpd8KwyBUyE1Sfn8uXMr6qidRXJeM4B
```

The Orbit Identifier is used in conjunction with an Orbit Method type
specification to resolve to a current Orbit Manifest. It is possible that
defaults or overrides are encoded into the Orbit Identifer, depending on the
Orbit Method type.

#### Orbit Roles
The keyholder known as the Orbit Commander may determine virtually all aspects
of the Orbit directly and indirectly by modifying the Orbit Manifest,
including:
- The list of public key-derived identifiers allowed to act as
  Orbit Commanders, Host Managers, Readers, Writers, Read Delegators, and Write
  Delegators.
- The data consistency strategy for content updates.
- The data consistency strategy for policy updates.
- The revocation strategy for capabilities.
- The supported authentication methods.

Host Managers may determine within the Orbit Manifest:
- A list of public key-derived identifiers mapped to one or more addresses,
  such as IPv4, IPv6, and `.onion`, which are then used by clients to resolve
  `kepler://` URIs.

Writers may determine within the Orbit Manifest:
- The value of the Orbit Manifest field containing the latest Merkle root of the
  Orbit's contents, which indicates the current valid state (if supported by the
  content consistency strategy).

Write Delegators may:
- Issue Write-scoped authorization capabilities via cryptographic signing.
- Determine within the Orbit Manifest the validity and revocation status of
  any Write Delegator issued capabilities such as through a revocation list,
  cryptographic accumulator, or OCSP-like protocol.

Readers are recognized by all Hosts to have:
- The implicit capability to list all of the Orbit's contents.
- The implicit capability to fetch all of the Orbit's contents.

Read Delegators may:
- Issue Read-scoped authorization capabilities via cryptographic signing.
- Determine within the Orbit Manifest the validity and revocation status of
  any Read Delegator issued capabilities such as through a revocation list,
  cryptographic accumulator, or OCSP-like protocol.

### Access Control

#### Capabilities
Other than the implicit permissions defined in the Orbit Manifest (such as
Readers' implicit ability to read from Hosts), permissioning is handled via
[capabilities](https://en.wikipedia.org/wiki/Capability-based_security).
Capabilities are scoped to constraints such as the action (read/write/list),
domain (contents/policy), and validity status (revocation list/expiration).

#### Authentication Methods
An Orbit's supported authentication methods are defined by the Orbit Commander,
and they must all be key-based.  For example, it is possible to support
cryptocurrency wallets with signing capabilities, such as ad hoc defined
specifications such as [EIP-712](https://eips.ethereum.org/EIPS/eip-712), or
arbitrarily padded signing with other ecosystem wallets. It is also possible to
authenticate with a [ZCAP](https://w3c-ccg.github.io/zcap-ld/) wrapped in a
[Verifiable
Presentation](https://w3c.github.io/vc-data-model/#presentations-0). Support
for a specific signing scheme is a matter of implementing an authentication
method module for it, which consists of mapping signatures over structured data
to Kepler capabilities, and then instructing the hosts to accept it.

For example, this [authentication
method](https://github.com/spruceid/kepler/blob/main/src/tz.rs) implements
support for signing via Tezos wallets, which prefixes all signed data with
`"Tezos Signed Message:"`.

### Hosts
Hosts are where users of the Orbit can go to get service.

Each Host uses a cryptographic keypair to serve as their core identifier and
authenticator. A Host may consist of one or many machines, IP addresses, or
other form of distribution. The keypair is used to demarcate a logical
separation useful to the Orbit, such as ownership, SLA, or region.

Ultimately, Kepler will support a variety of storage systems, namely IPFS, but
also AWS S3, GCP Cloud Storage, Azure Blob Storage, Network File Systems, and
even user-friendly REST API-supporting services such as Dropbox or Box.com.
Different use cases have different backend requirements.

The storage medium will typically matter less to the system, which focuses on
interfaces which only requires upon reading, writing, and listing of content
objects against a generic storage backend. However, people care a lot more
about the storage medium where data are held, such as to comply with
regulations (GDPR/CCPA/MyData/HIPAA/PCI/etc.), reduce network latency through
regional guarantees, and achieve high performance by specifying disk types. 

### Geometry
Orbits can be arranged in a geometry, or series of relationships. For example,
- ``Orbit3 <- Orbit1{R} `Geometry.CopyOnWrite` Orbit2{RW}``: `Orbit3` is
  constructed with the read-only `Orbit1` and read/write-capable `Orbit2`. When
  a user writes to `Orbit3`, the changes are actually captured in `Orbit2`, but
  to them, it looks as if they have modified `Orbit1`.
- ``Orbit6 <- Geometry.Compose [Orbit4, Orbit5]``: `Orbit6` is a read-only
  amalgamation of `Orbit4` and `Orbit5`.
- ``Orbit8 <- Orbit6@fe728f `Geometry.CopyOnWrite` Orbit7{RW}``: `Orbit8` is a
  copy-on-write of a specific version of `Orbit6`. When `Orbit6`'s underlying
  Orbits change, it does not affect `Orbit8` because it was versioned.
- ``Orbit11 <- Orbit9 `Geometry.Metadata` Orbit10``: `Orbit11` is `Orbit9`
  augmented with one-to-one metadata stored in `Orbit10`. The Orbit Commander
  may want to add the further requirement that `Orbit9` and `Orbit10` share the
  same set of Hosts to ensure performance.
- ``Orbit14 <- Orbit12 `Geometry.StreamBuffer[4MB]` Orbit13``: `Orbit14` is
  `Orbit12`, an Orbit with support for streaming content objects, buffered per
  stream with a fixed size limit using `Orbit13`. This allows for replay of
  bytestreams up to the size limit.
