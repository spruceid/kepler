use core::future::IntoFuture;
use core::time::Duration;
use futures::{
    channel::{mpsc, oneshot},
    future::{select, Either},
    io::{AsyncRead, AsyncWrite},
    sink::{Sink, SinkExt},
    stream::{Stream, StreamExt},
};
use libp2p::{
    identify::Config as OIdentifyConfig,
    identity::{Keypair, PublicKey},
    swarm::{
        behaviour::toggle::Toggle, ConnectionHandler, IntoConnectionHandler, NetworkBehaviour,
        Swarm, SwarmEvent,
    },
};
use std::fmt::Display;

pub mod behaviour;
pub mod relay;
pub mod transport;

pub const PROTOCOL_VERSION: &'static str = "kepler/0.1.0";

pub trait BehaviourConfig {
    type Error;
    type Behaviour: NetworkBehaviour;
    fn build<T>(
        self,
        keypair: Keypair,
        transport: T,
    ) -> Result<Swarm<Self::Behaviour>, Self::Error>;
}

#[async_trait]
pub trait Logic<B>
where
    B: NetworkBehaviour,
{
    type Error;
    type Message;
    type Event;
    async fn process_message(
        &mut self,
        swarm: &mut Swarm<B>,
        event: Self::Event,
    ) -> Result<Option<Self::Event>, Self::Error>;
    async fn process_swarm(
        &mut self,
        swarm: &mut Swarm<B>,
        event: SwarmEvent<
            B::OutEvent,
            <<B::ConnectionHandler as IntoConnectionHandler>::Handler as ConnectionHandler>::Error,
        >,
    ) -> Result<Option<Self::Event>, Self::Error>;
}

pub async fn launch<L, B>(
    logic: L,
    swarm: Swarm<B>,
    messages: impl Stream<Item = L::Message> + Unpin,
    events: impl Sink<L::Event> + Unpin,
) -> Result<(), L::Error>
where
    L: Logic<B>,
    B: NetworkBehaviour,
{
    loop {
        match select(messages.next(), swarm.next()).await {
            // if the swarm or the channel are closed, close the relay
            Either::Right((None, _)) | Either::Left((None, _)) => {
                break;
            }
            // process message
            Either::Left((Some(m), _)) => match logic.process_message(&mut swarm, m).await? {
                Some(e) => events.send(e).await,
                _ => (),
            },
            // process swarm event
            Either::Right((Some(m), _)) => match logic.process_swarm(&mut swarm, m).await? {
                Some(e) => events.send(e).await,
                _ => (),
            },
        };
    }
    Ok(())
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct IdentifyConfig {
    protocol_version: String,
    initial_delay: Duration,
    interval: Duration,
    push_listen_addr_updates: bool,
    cache_size: usize,
}

impl IdentifyConfig {
    pub fn protocol_version(&mut self, i: impl Into<String>) -> &mut Self {
        self.protocol_version = i.into();
        self
    }
    pub fn initial_delay(&mut self, i: impl Into<Duration>) -> &mut Self {
        self.initial_delay = i.into();
        self
    }
    pub fn interval(&mut self, i: impl Into<Duration>) -> &mut Self {
        self.interval = i.into();
        self
    }
    pub fn push_listen_addr_updates(&mut self, i: impl Into<bool>) -> &mut Self {
        self.push_listen_addr_updates = i.into();
        self
    }
    pub fn cache_size(&mut self, i: impl Into<usize>) -> &mut Self {
        self.cache_size = i.into();
        self
    }
}

impl Default for IdentifyConfig {
    fn default() -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION.into(),
            initial_delay: Duration::from_millis(500),
            interval: Duration::from_secs(300),
            push_listen_addr_updates: false,
            cache_size: 0,
        }
    }
}

impl IdentifyConfig {
    fn to_config(self, key: PublicKey) -> OIdentifyConfig {
        OIdentifyConfig::new(self.protocol_version, key)
            .with_initial_delay(self.initial_delay)
            .with_interval(self.interval)
            .with_push_listen_addr_updates(self.push_listen_addr_updates)
            .with_cache_size(self.cache_size)
    }
}

impl From<OIdentifyConfig> for IdentifyConfig {
    fn from(c: OIdentifyConfig) -> Self {
        IdentifyConfig {
            protocol_version: c.protocol_version,
            initial_delay: c.initial_delay,
            interval: c.interval,
            push_listen_addr_updates: c.push_listen_addr_updates,
            cache_size: c.cache_size,
        }
    }
}
