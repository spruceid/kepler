use crate::{
    orbit::AbortOnDrop,
    p2p::{transport::IntoTransport, IdentifyConfig},
};
use anyhow::Result;
use futures::{
    channel::{mpsc, oneshot},
    future::{select, Either},
    io::{AsyncRead, AsyncWrite},
    sink::SinkExt,
    stream::{iter, StreamExt},
};
use libp2p::{
    autonat::Behaviour as AutoNat,
    core::{identity::Keypair, upgrade, Multiaddr, PeerId},
    identify::Behaviour as Identify,
    mplex, noise,
    ping::Behaviour as Ping,
    relay::v2::relay::Relay,
    swarm::{Swarm, SwarmBuilder},
    yamux, NetworkBehaviour,
};

pub use builder::Config;

pub type RelaySwarm = Swarm<Behaviour>;

#[derive(Clone, Debug)]
pub struct RelayNode {
    id: PeerId,
    sender: mpsc::Sender<Message>,
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
    ListenOn(Vec<Multiaddr>, oneshot::Sender<Result<()>>),
}

impl RelayNode {
    pub fn id(&self) -> &PeerId {
        &self.id
    }
    pub async fn get_addresses(&mut self) -> Result<Vec<Multiaddr>> {
        let (s, r) = oneshot::channel();
        self.sender.send(Message::GetAddresses(s)).await?;
        Ok(r.await?)
    }

    pub async fn listen_on(&mut self, addr: impl IntoIterator<Item = Multiaddr>) -> Result<()> {
        let (s, r) = oneshot::channel();
        self.sender
            .send(Message::ListenOn(addr.into_iter().collect(), s))
            .await?;
        r.await?
    }
}

mod builder {
    use super::*;
    use derive_builder::Builder;
    use libp2p::{
        autonat::Config as AutoNatConfig, core::Transport, identity::PublicKey,
        ping::Config as PingConfig, relay::v2::relay::Config as RelayConfig,
    };

    #[derive(Builder)]
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

        pub fn launch<T>(self, keypair: Keypair, transport: T) -> Result<RelayNode, T::Error>
        where
            T: IntoTransport,
            T::T: 'static + Send + Unpin,
            T::Error: 'static + std::error::Error + Send + Sync,
            <T::T as Transport>::Output: 'static + AsyncRead + AsyncWrite + Unpin + Send,
            <T::T as Transport>::Error: 'static + Send + Sync,
            <T::T as Transport>::Dial: Send,
            <T::T as Transport>::ListenerUpgrade: Send,
        {
            let local_public_key = keypair.public();
            let id = local_public_key.to_peer_id();
            let b = self.build(local_public_key);
            let (sender, mut reciever) = mpsc::channel(100);
            let r = RelayNode { id, sender };

            let mut swarm = SwarmBuilder::new(
                transport
                    .into_transport()?
                    .upgrade(upgrade::Version::V1)
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

            tokio::spawn(async move {
                loop {
                    match select(reciever.next(), swarm.next()).await {
                        // if the swarm or the channel are closed, close the relay
                        Either::Right((None, _)) | Either::Left((None, _)) => {
                            break;
                        }
                        // process command
                        Either::Left((Some(e), _)) => match e {
                            Message::ListenOn(a, s) => {
                                // try listen on each given address
                                match a.into_iter().try_fold(Vec::new(), |mut listeners, addr| {
                                    match swarm.listen_on(addr) {
                                        Ok(l) => {
                                            listeners.push(l);
                                            Ok(listeners)
                                        }
                                        Err(e) => Err((e, listeners)),
                                    }
                                }) {
                                    Ok(_) => s.send(Ok(())),
                                    // if one fails, roll back all of them
                                    Err((e, listeners)) => {
                                        for l in listeners {
                                            swarm.remove_listener(l);
                                        }
                                        s.send(Err(e.into()))
                                    }
                                }
                                .map_err(|_| anyhow!("failed to return listening result"))?;
                            }
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
