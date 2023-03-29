use crate::p2p::{
    behaviour::{BaseBehaviour, Behaviour},
    transport::IntoTransport,
    IdentifyConfig,
};
use futures::{
    channel::{mpsc, oneshot},
    future::{select, Either},
    io::{AsyncRead, AsyncWrite},
    sink::SinkExt,
    stream::StreamExt,
};
use libp2p::{
    autonat::{Behaviour as AutoNat, Config as AutoNatConfig},
    core::{upgrade, Transport},
    dcutr::Behaviour as Dcutr,
    gossipsub::{
        Behaviour as Gossipsub, Config as GossipsubConfig, ConfigBuilder as GossipsubConfigBuilder,
        MessageAuthenticity, ValidationMode,
    },
    identify::Behaviour as Identify,
    identity::{Keypair, PeerId},
    kad::{
        record::store::{MemoryStore, MemoryStoreConfig, RecordStore},
        Kademlia, KademliaConfig,
    },
    mplex, noise,
    ping::{Behaviour as Ping, Config as PingConfig},
    relay::client::{new, Behaviour as Client},
    swarm::{Swarm, SwarmBuilder},
    yamux,
};
use thiserror::Error;

#[derive(Debug, Clone, Default)]
pub struct BehaviourConfig<KSC = MemoryStoreConfig> {
    identify: IdentifyConfig,
    ping: PingConfig,
    gossipsub: GossipsubConfig,
    kademlia: KademliaConfig,
    kademlia_store: KSC,
    autonat: AutoNatConfig,
    relay: bool,
}

impl<KSC> BehaviourConfig<KSC> {
    pub fn new(ksc: impl Into<KSC>) -> Self {
        Self {
            identify: Default::default(),
            ping: Default::default(),
            gossipsub: Default::default(),
            kademlia: Default::default(),
            kademlia_store: ksc.into(),
            autonat: Default::default(),
            relay: Default::default(),
        }
    }
    pub fn identify(&mut self, i: impl Into<IdentifyConfig>) -> &mut Self {
        self.identify = i.into();
        self
    }
    pub fn ping(&mut self, i: impl Into<PingConfig>) -> &mut Self {
        self.ping = i.into();
        self
    }
    pub fn gossipsub(&mut self, i: impl Into<GossipsubConfig>) -> &mut Self {
        self.gossipsub = i.into();
        self
    }
    pub fn kademlia(&mut self, i: impl Into<KademliaConfig>) -> &mut Self {
        self.kademlia = i.into();
        self
    }
    pub fn kademlia_store(&mut self, i: impl Into<KSC>) -> &mut Self {
        self.kademlia_store = i.into();
        self
    }
    pub fn autonat(&mut self, i: impl Into<AutoNatConfig>) -> &mut Self {
        self.autonat = i.into();
        self
    }
    pub fn relay(&mut self, i: impl Into<bool>) -> &mut Self {
        self.relay = i.into();
        self
    }
    fn build<KS>(
        self,
        keypair: Keypair,
        relay: Option<Client>,
    ) -> Result<Behaviour<KS>, OrbitBehaviourBuildError>
    where
        KSC: RecordStoreConfig<KS>,
        KS: RecordStore + Send + 'static,
    {
        let peer_id = keypair.public().to_peer_id();
        Ok(Behaviour {
            exchange: todo!(),
            base: BaseBehaviour {
                identify: Identify::new(self.identify.to_config(keypair.public())),
                ping: Ping::new(self.ping),
                gossipsub: Gossipsub::new(
                    MessageAuthenticity::Signed(keypair),
                    GossipsubConfigBuilder::from(self.gossipsub)
                        // always ensure validation
                        .validation_mode(ValidationMode::Strict)
                        .build()
                        .map_err(OrbitBehaviourBuildError::Gossipsub)?,
                )
                .map_err(OrbitBehaviourBuildError::Gossipsub)?,
                relay: relay.into(),
                kademlia: Kademlia::with_config(
                    peer_id,
                    self.kademlia_store.init(peer_id),
                    self.kademlia,
                ),
                dcutr: Dcutr::new(peer_id),
                autonat: AutoNat::new(peer_id, self.autonat),
            },
        })
    }
    pub fn launch<T, KS>(
        self,
        keypair: Keypair,
        transport: T,
    ) -> Result<(), OrbitLaunchError<T::Error>>
    where
        T: IntoTransport,
        T::T: 'static + Send + Unpin,
        T::Error: 'static + std::error::Error + Send + Sync,
        <T::T as Transport>::Output: 'static + AsyncRead + AsyncWrite + Unpin + Send,
        <T::T as Transport>::Error: 'static + Send + Sync,
        <T::T as Transport>::Dial: Send,
        <T::T as Transport>::ListenerUpgrade: Send,
        KS: RecordStore + Send + 'static,
    {
        let local_public_key = keypair.public();
        let id = local_public_key.to_peer_id();
        let transport = transport.into_transport()?;
        let (transport, behaviour) = if self.relay {
            let (t, b) = new(id);
            (transport.or_transport(t), self.build(keypair, Some(b))?)
        } else {
            (transport, self.build(keypair, None)?)
        };

        let mut swarm = SwarmBuilder::with_tokio_executor(
            transport
                .upgrade(upgrade::Version::V1)
                // TODO replace with AWAKE protcol (or similar)
                .authenticate(noise::NoiseAuthenticated::xx(&keypair).unwrap())
                .multiplex(upgrade::SelectUpgrade::new(
                    yamux::YamuxConfig::default(),
                    mplex::MplexConfig::default(),
                ))
                .timeout(std::time::Duration::from_secs(20))
                .boxed(),
            behaviour,
            id,
        )
        .build();
        Ok(())
    }
}

#[derive(Error, Debug)]
pub enum OrbitBehaviourBuildError {
    #[error("{0}")]
    Gossipsub(&'static str),
}

#[derive(Error, Debug)]
pub enum OrbitLaunchError<T> {
    #[error(transparent)]
    Config(#[from] OrbitBehaviourBuildError),
    #[error(transparent)]
    Transport(T),
}

pub trait RecordStoreConfig<S>
where
    S: RecordStore + Send + 'static,
{
    fn init(self, id: PeerId) -> S;
}

impl RecordStoreConfig<MemoryStore> for MemoryStoreConfig {
    fn init(self, id: PeerId) -> MemoryStore {
        MemoryStore::with_config(id, self)
    }
}
