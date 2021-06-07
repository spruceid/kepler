---
title: Kepler: The New S3 - Self-Sovereign Storage
classoption:
- twocolumn
authors:
- Wayne Chang (wayne@spruceid.com)
---

\title{Kepler: The New S3 - Self-Sovereign Storage}

\begin{abstract}
Kepler is a system for user-controlled decentralized storage with
permissioning. It is built atop existing storage backends and blockchains,
using cryptographic key pairs and capabilities for access control. It offers
powerful data mapping and compute functionality inspired by the FreeBSD's GEOM.
It is being implemented as open source.
\end{abstract}

# Introduction
Kepler is a decentralized and permissioned storage system that organizes data
into user-controlled overlays called Orbits. Orbits can be configured with
policies such as permissioning, replication, and content resolution. Access
control is based on cryptographic key pairs and programmable capabilities.
Orbits may use smart contracts for baseline availability and root authority.
Kepler's design goal is to provide user-controlled storage with highly
configurable availability, access control, and data transformations for any
content data, including claims, large datasets, multimedia, and streams. It is
in development at Spruce Systems, Inc. as open source software under the Apache
2.0 license.

## Focus Areas

Within the past ten years, we have seen the rapid rise of distributed storage
systems that can run peer-to-peer across untrusted nodes, notably BitTorrent
and IPFS. With the growing popularity of these systems, we now face new
challenges currently addressed only with ad hoc solutions. Kepler seeks to
solve the most important challenges in a standard way that also ensures user
control and decentralization, specifically:

- **Data Availability**: How do we ensure that data remain available as is
necessary, reside in appropriate physical or logical places, resolve with
acceptable latency, and can handle traffic requirements? For example, a media
content distribution network (CDN) may require servers with acceptable uptime
or hardware specifications to serve its content. A consumer finance app may be
required to keep records in specific countries to comply with GDPR.

- **Access Control**: How do we guarantee that only the right parties can
access the data in the right ways? Consider a creator who wishes to mint a
Non-Fungible Token (NFT) such that only the current owner can access to the
original high resolution image or live video stream. A patient may want only
their current primary care physician and active specialists to have read and
write access to the relevant parts of their health records.

- **Data Transformations**: What new useful data manipulations and computations
are possible using guarantees around scoping, availability, and access control
for decentralized storage? For example, a user may require a virtual decrypted
view into their personal data store that supports read and write. The user
might share this read-only version of their decrypted data store to a trusted
friend, who then wants faux write access by storing just their own changes in a
different data store for which they do have write access.

While many distributed stores model all data as part of the same global set,
Kepler wraps a scoped set of data into an Orbit and is able to provide
guarantees for its data retrieval rules, replication, permissioning, and data
transformations. Kepler Orbits are a composible new primitive that can enable
an entire new class of data applications that do not sacrifice a user's control
over their data while also achieving decentralization and high performance.

# Background

## Decentralized and Permissioned Storage

## IPFS 

## BitTorrent

## Tahoe-LAFS

# Kepler Design

## Orbit Manifest
The Orbit Manifest is a digital document that describes all important aspects
about the the Orbit's data and policies, namely:

- The latest stable content state (e.g., via Merkle root hash).
- The base access control list.
- The list of hosts from which clients list, fetch, and update content.
- The revocation strategy and validity status for capabilities.
- The data consistency strategies for content and policy.
- The supported authentication methods.

This is an example of what an Orbit Manifest might look like after it is
fetched from its data store compact representation and formatted into JSON:

```

```

The Orbit Manifest can live:

- In a smart contract on a blockchain, such as Ethereum.
- In an updatable distributed data store with high availability, such as Apache
  ZooKeeper.
- On a permissioned DHT-like service with update capabilities such as ipfs-log
  or DHT Mutable Items.
- Within web object storage service such as AWS S3 or MinIO.

Ultimately, the storage medium for the Orbit Manifest will depend on
requirements for its discoverability, performance, permissioning, and
resilience. For example, many smart contract languges would allow an Orbit
Manifest's base permissioning to be enforced at the blockchain VM level, such
as demonstrated by this this Vyper snippet:

```
```

## Orbit Identifiers and Orbit Methods

To resolve an Orbit Manifest, users start with an Orbit Identifier, which are
URI `host` field with matrix parameters that describe how to find the Orbit
Manifest or set default states. These matrix parameters are interpreted
differently depending on the Orbit Method specified in the `host` field. Each
Orbit Method must be fully specified and registered, similar to W3C DID
methods.

For example, Orbit Method based on Ethereum might look like the following
(broken into lines for readability):

### Example 1: Smart Contract-defined Orbit Manifest
```
eth;address=0x27ae27110350b98d564b9a3ffd31baebc82d878f
```
- `eth` refers to the Orbit Method.
- `address` is the Ethereum address, in this case, a smart contract. The Orbit
  Method will specify how to interpret the smart contract data into an Orbit
  Manifest and default values if it does not exist.

### Example 2: Smart Contract-defined with Implicit Defaults
```
eth;address=0x27ae27110350b98d564b9a3ffd31baebc82d878f;host=55.13.9.4;\
    merkle-root=1220417b6443542e..22ab11d2589a8
```
- `eth` refers to the Orbit Method.
- `address` is the Ethereum address, in this case, a smart contract.
- `host` is a repeated parameter specifying a default host list used by clients
  to resolve content. This may be overidden by values in the smart contract, if
  one exists.
- `merkle-root` is the Merkle root of the orbit's contents. This may be
  overidden by values in the smart contract, if one exists.

### Example 3: External Account-based with Implicit Default
```
eth;address=0x89205A3A3b2A698e67bf7f01ED13B2108B2c43e7;host=55.13.9.4;\
    index=0;merkle-root=1220417b6443542e..22ab11d2589a8
```
- `address` is the Ethereum address, in this case, an external account. The
  Orbit Method may specify that an indexer must be used to look up a smart
  contract deployed by this account and use its address as a default value for
  several fields.
- `host` is the same as the previous example.
- `index` specifies which conforming smart contract to use, as an account may
  deploy multiple conforming smart contracts.
- `merkle-root` is the same as the previous example.

## Kepler URLs

The Orbit Identifier may be combined with a Content Identifier expressed in
`multibase` to form a Kepler URL prefixed with the `kepler://` scheme.

\subsubsection{Orbit Roles}

\subsubsection{Smart Contracts}

\subsection{Access Control}

\subsubsection{Decentralized Identifiers}

\subsubsection{Programmable Capabilities}

\subsubsection{Linked Data Proofs}

\subsubsection{cryptoscript}

\subsection{Hosts}

\subsubsection{Identity}

\subsubsection{Replication}

\subsubsection{Storage Backends}

\subsubsection{Host Requirements}

\subsection{Orbit Relationships}

\subsubsection{Transformations}

\subsubsection{Gateways}

\subsection{Use Cases}

\section{Acknowledgements}

https://tahoe-lafs.org/trac/tahoe-lafs/browser/docs/about.rst
https://www.w3.org/DesignIssues/MatrixURIs.html
