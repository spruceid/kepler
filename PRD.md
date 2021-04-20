# Kepler
Product Requirements Document

Author: Wayne Chang <wayne@spruceid.com>
Date: October 9th, 2020

Kepler is a decentralized data storage system built as a network of storage
nodes and data overlays called orbits. Orbits can be replicated, permissioned,
and accessed according to their own policies.

Many decentralized networks rely on centralized storage providers such as AWS
S3, GitHub, or IPFS instances hosted by a single company. They do this to meet
file hosting expectations of high public availability, low latency, reasonable
cost. With Kepler, we seek to achieve decentralization through local
consensus, with each orbit adhering to flexible governance criteria, for
example:

- A group of friends agree to collectively host an orbit.
- An enterprise uses legal contracts to bind partners to orbit SLAs.
- A decentralized network binds orbits to cryptoeconomic validation models,
  such as by using Filecoin or Sia.

Furthermore, it is necessary to provide controls across the entire network to
govern deletion of select data to comply with regulations such as GDPR or to
redress spam attacks. We imagine an add-on accounting and billing system built
to support key-based authentication, cryptocurrencies, and/or GNU Taler to
allow for long term sustainability.

## Why
- Owners of decentralized service accounts (federated networks, blockchains,
  secure scuttlebutt, etc.) require public data stores that are highly
  available, affordable, reasonably censorship resistant, and supporting
  locally hosted instances if desired.
- Decentralized ecosystems prefer a sustainable ecosystem of reliable storage
  nodes over a single centralized service provider.
- We need to comply with regional regulations with respect to PII and illegal
  data across the entire network of nodes.
- We need a way to allow nodes to permissionlessly enter and leave the network
  and manage incentives so that the network is stable as it grows.

## User Profiles
- **dApp developer** who needs to host their dApp, store additional assets
  used in their dApp such as user-submitted NFT images, publish their source
  code, or host a PDF of passing software audits.
- **Digital asset issuer** who wants to attach publicly-facing information to
  their pegged tokens, such as deposit receipts for a gold bar or an audit
  report for a fiat reserves backing stablecoin issuance.
- **Issuer of Verifiable Credentials** who needs storage for publicly-facing
  data schemas and verifiable credentials in both human and machine readable
  formats, revocation lists, or evidence related to verifiable credentials
  such as large binaries that can be evaluated to reproduce results.

## MVP Requirements
- A keypair should be all you need to get started with data storage. The user
  should be able to use this storage service with just a wallet and dApp
  without ever leaving the dApp, even if you haven’t used the service before.
- Kepler should work fine with regular Web 2 applications too, such as taking the
  form of a storage module in Rails or SDK usable by a Go microservice.
- The nodes must be permissioned with authentication and authorization for
  storage, and we should figure out how to prevent basic spam attacks such as
  generating a billion keypairs and uploading 5 MB each from a few IP
  addresses.
- We need a way to remove PII and illegal content from the network to comply with laws.
- It should be straightforward to run a federated Kepler node on standard cloud
  hosting.
- We should be able to grant people additional storage due to certain status in
  the community, for example:
    - They have a certain number of mainnet blockchain transactions (Query an
      indexer)
    - They present W3C Verifiable Credentials by the correct issuer and data
      values (Spruce to provide libraries that can handle this)
    - Their account has deployed or used a popular and well-used smart contract
      (Query an indexer)


## Long Term Considerations
- Users should also be able to pay for storage with cryptocurrencies such as
  XTZ, eventually to be collected by the people who run nodes.
- Kepler should be equipped with different "orbits" which are file overlays and
  software-enforced policies for a specific community. For example, there may
  be a dApp Orbit dedicated to hosting dApp assets, or a Public Records Orbit
  which hosts copies of S1 filings from the SEC’s website among other public
  documents of note. A Kepler node can belong to one or many orbits, and as a
  result adopts the rules and economic reward structure of that orbit.
- We should have tooling to deploy, monitor, and manage Kepler nodes and
  orbits. Ideally, a cloud subaccount API key would be all that is required to
  stand one up, and orbits may be selected from the administration panel.
- We should support partial orbits, in which only a fraction of the files from
  an orbit are stored due to space constraints. The asset durabilities should
  be examinable when analyzing orbits.
- We should explore "copy-on-write" orbits which resolve to another orbit upon
  file requests, but are able to keep a running list of changes versus the
  reference orbit.
- We should explore "composed" orbits which are represented by a list of
  rank-ordered orbits where file resolution proceeds sequentially until a hit
  or list end.
- We should support Bittorrent-style downloading across many peers at the same
  time.
- We should be able to use a DAO to govern orbits to delete or deactivate
  illegal files, sticklers of censorship resistance can run their own nodes in
  "no-delete" mode and host the files at their own risk, but it will not be
  officially recognized by the orbit.

## Integration Considerations
- Kepler should support a gateway where an orbit can be designated as public
  and then exposed to anyone on the internet, like a Tor exit node. The gateway
  should eventually be able to load balance requests across nodes in the orbit.
- A cryptocurrency wallet such as Kukai or Temple be able to authorize CRUD
  requests against orbits, such as by using the `Tezos Signed Message`
  functionality of the wallets. This allows dApps to access Kepler storage.
- W3C Verifiable Credentials or Verifiable Presentations may be presented to
  the nodes to authenticate or authorize.
- We should support a variety of storage and replication backends, including
  IPFS, S3, NFS, and RocksDB. These should be modular and selectable by the end
  user.
- For discovery of orbit hosts and latest content versions, we should consider
  the use of a DHT and smart contracts to anchor the launch data. DHTs may be
  permissioned in the case of a private orbit. Launch data contains the latest
  content version, perhaps a Merkle Patricia trie root hash, and active nodes
  in the orbit that can serve requests.

## MVP Scoping
We will not satisfy all requirements above on the first pass. Instead, we
should figure out how to deploy something by the Q1 of 2021 that can be used in
production with the major use cases above. The most important objective is
usable storage that community members can rely on for their projects. We will
describe the minimal “happy case” user workflow that must be implemented first,
then possibly iterated upon to eventually meet all requirements.

### Kepler Node Administrator
A Kepler Node Administrator is typically the CTO or senior engineer at a
small engineering firm such as TQ, Baking Bad, ECAD Labs, or Spruce, that
wishes to run and use the Kepler storage network. They are technically
sophisticated but have limited time and developer resources to spend setting up
a node and making it run reliably.

For the MVP, they want to be able to
- Run an instance of Kepler on their cloud provider of choice in an isolated
  but Internet-connected environment, such as on AWS, Azure, GCP, or Digital
  Ocean.
- Not have to worry about cloud security settings, data storage abuse, takedown
  notices, or skyrocketing costs.
- Be able to access their Kepler node through an administration panel and
  monitor statistics such as peering, storage usage, uptime, bandwidth, and
  more.

## Kepler Storage User
A Kepler Storage User has a Tezos account and wants to store files on the
Kepler storage network (KSN). For the MVP, they want to be able to:
- Deploy their dApp to KSN including its frontend web assets.
- Store files on KSN from a dApp without leaving the dApp.
- Store arbitrary files on KSN from the command line.
- Receive statistics on their hosted files, such as durability (redundancy),
  bandwidth statistics, and more.
- Login to their KSN management portal and view their data and storage quotas.
- View the reasons for increases in storage quota, such as having a certain
  number of mainnet transactions associated with their account.

# Kepler Network Administrator
The Kepler Network Administrator responds to takedown requests and fights spam
on behalf of all nodes. It is the centralized governance actor that we hope to
eventually decentralize with BaseDAO. We expect a single party, possibly
whoever builds the system, to play this role for now with oversight from the
network members. Specifically, they:
- Respond to DMCA takedown requests, GDPR requests, or similar and proactively
  fight spam attacks/illegal content.
- Set up and maintain the core software projects.
- Monitor network-level health, including file durability.
- Run a node cluster of last resort to add to high availability guarantees.




