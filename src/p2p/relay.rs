use crate::{
    orbit::AbortOnDrop,
    p2p::{transport::IntoTransport, IdentifyConfig},
};
use anyhow::Result;
use futures::{
    channel::{mpsc, oneshot},
    future::{select, Either},
    io::{AsyncRead, AsyncWrite},
    sink::{Sink, SinkExt},
    stream::{iter, Stream, StreamExt},
};
use libp2p::{
    autonat::Behaviour as AutoNat,
    core::{
        identity::Keypair, multiaddr::multiaddr, transport::upgrade::Builder, upgrade, Multiaddr,
        PeerId,
    },
    identify::Behaviour as Identify,
    mplex, noise,
    ping::Behaviour as Ping,
    relay::v2::relay::Relay,
    swarm::{Swarm, SwarmBuilder, NetworkBehaviour},
    yamux,
};

pub use builder::Config;

pub type RelaySwarm = Swarm<Behaviour>;

#[derive(Clone, Debug)]
pub struct RelayNode {
    id: PeerId,
    sender: mpsc::Sender<Message>,
    port: u16,
}

#[derive(NetworkBehaviour)]
pub struct Behaviour {
    identify: Identify,
    ping: Ping,
    relay: Relay,
    autonat: AutoNat,
}

#[derive(Debug)]
pub enum Message {
    GetAddresses(oneshot::Sender<Vec<Multiaddr>>),
    ListenOn(Multiaddr),
}

impl RelayNode {
    pub fn id(&self) -> &PeerId {
        &self.id
    }
    async fn get_addresses(&mut self) -> Result<Vec<Multiaddr>> {
        let (s, r) = oneshot::channel();
        self.sender.send(Message::GetAddresses(s)).await?;
        Ok(r.await?)
    }

    async fn listen_on(&mut self, addr: impl IntoIterator<Item = &Multiaddr>) -> Result<()> {
        Ok(self
            .sender
            .send_all(&mut iter(
                addr.into_iter().map(|a| Ok(Message::ListenOn(a.clone()))),
            ))
            .await?)
    }

    fn _internal(port: u16) -> Multiaddr {
        multiaddr!(Memory(port))
    }

    pub fn internal(&self) -> Multiaddr {
        Self::_internal(self.port)
    }

    fn _external(port: u16) -> Multiaddr {
        multiaddr!(Ip4([127, 0, 0, 1]), Tcp(port))
    }

    pub fn external(&self) -> Multiaddr {
        Self::_external(self.port)
    }
}

mod builder {
    use super::*;
    use derive_builder::Builder;
    use libp2p::{
        autonat::Config as AutoNatConfig, core::Transport, identity::PublicKey,
        ping::Config as PingConfig, relay::v2::relay::Config as RelayConfig,
    };

    #[derive(Builder, Debug)]
    #[builder(
        build_fn(skip),
        setter(into),
        name = "Config",
        derive(Debug),
        pattern = "owned"
    )]
    pub struct BehaviourConfigDummy {
        #[builder(field(type = "IdentifyConfig"))]
        identify: IdentifyConfig,
        #[builder(field(type = "PingConfig"))]
        ping: PingConfig,
        #[builder(field(type = "RelayConfig"))]
        relay: RelayConfig,
        #[builder(field(type = "AutoNatConfig"))]
        autonat: AutoNatConfig,
    }

    impl Config {
        fn build(self, pubkey: PublicKey) -> Behaviour {
            let peer_id = pubkey.to_peer_id();
            Behaviour {
                identify: Identify::new(self.identify.to_config(pubkey)),
                ping: Ping::new(self.ping),
                relay: Relay::new(peer_id, self.relay),
                autonat: AutoNat::new(peer_id, self.autonat),
            }
        }

        pub fn launch<T>(
            self,
            keypair: Keypair,
            transport: Builder<T>,
            port: u16,
        ) -> Result<RelayNode>
        where
            T: Transport + Send,
            T::Output: AsyncRead + AsyncWrite + Unpin + Send,
            T::Error: Send + Sync,
            T: Unpin,
            T::Dial: Send,
            T::ListenerUpgrade: Send,
        {
            let local_public_key = keypair.public();
            let id = local_public_key.to_peer_id();
            let b = self.build(local_public_key);
            let (sender, mut reciever) = mpsc::channel(100);
            let r = RelayNode { id, sender, port };

            let mut swarm = SwarmBuilder::with_tokio_executor(
                transport
                    .authenticate(noise::NoiseAuthenticated::xx(&keypair).unwrap())
                    .multiplex(upgrade::SelectUpgrade::new(
                        yamux::YamuxConfig::default(),
                        mplex::MplexConfig::default(),
                    ))
                    .timeout(std::time::Duration::from_secs(20))
                    .boxed(),
                b,
                id,
            )
            .build();

            swarm.listen_on(r.external())?;
            swarm.listen_on(r.internal())?;

            tokio::spawn(async move {
                loop {
                    match select(reciever.next(), swarm.next()).await {
                        // if the swarm or the channel are closed, close the relay
                        Either::Right((None, _)) | Either::Left((None, _)) => {
                            break;
                        }
                        // process command
                        Either::Left((Some(e), _)) => match e {
                            Message::ListenOn(a) => swarm.listen_on(a).map(|_| ())?,
                            Message::GetAddresses(s) => {
                                s.send(swarm.listeners().map(|a| a.clone()).collect())
                                    .map_err(|_| anyhow!("failed to return listeners"))?;
                            }
                        },
                        Either::Right((Some(e), _)) => {
                            // process swarm event
                        }
                    }
                }
                Result::<(), anyhow::Error>::Ok(())
            });
            Ok(r)
        }
    }
}

#[cfg(test)]
pub mod test {}
