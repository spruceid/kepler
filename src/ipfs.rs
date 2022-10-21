use anyhow::Result;
use kepler_lib::libipld::{
    block::Block as OBlock,
    cid::{multibase::Base, Cid},
    multihash::Code,
    raw::RawCodec,
    store::DefaultParams,
};
use libp2p::{core::transport::MemoryTransport, futures::TryStreamExt, swarm::Swarm as TSwarm};
use std::{future::Future, sync::mpsc::Receiver};

use super::{cas::ContentAddressedStorage, codec::SupportedCodecs};
use crate::{
    config,
    kv::behaviour::{Behaviour, Event as BehaviourEvent},
    storage::{Repo, StorageUtils},
};

pub type KeplerParams = DefaultParams;
pub type Block = OBlock<KeplerParams>;
pub type Swarm = TSwarm<Types>;
