use crate::p2p::{transport::IntoTransport, IdentifyConfig};
use anyhow::Result;
use futures::{
    channel::{mpsc, oneshot},
    future::{select, Either},
    io::{AsyncRead, AsyncWrite},
    sink::SinkExt,
    stream::StreamExt,
};
use libp2p::{
    autonat::{Behaviour as AutoNat, Config as AutoNatConfig},
    core::{identity::Keypair, upgrade, Multiaddr, Transport},
    identify::Behaviour as Identify,
    identity::PublicKey,
    mplex, noise,
    ping::{Behaviour as Ping, Config as PingConfig},
    relay::{Behaviour as Relay, Config as RelayConfig},
    swarm::{NetworkBehaviour, SwarmBuilder},
    yamux, PeerId,
};

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
enum Message {
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

#[derive(Debug)]
pub struct Config {
    identify: IdentifyConfig,
    ping: PingConfig,
    relay: RelayConfig,
    autonat: AutoNatConfig,
    channel_size: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            identify: IdentifyConfig::default(),
            ping: PingConfig::default(),
            relay: RelayConfig::default(),
            autonat: AutoNatConfig::default(),
            channel_size: 100,
        }
    }
}

impl Config {
    pub fn identify(&mut self, i: impl Into<IdentifyConfig>) -> &mut Self {
        self.identify = i.into();
        self
    }
    pub fn ping(&mut self, i: impl Into<PingConfig>) -> &mut Self {
        self.ping = i.into();
        self
    }
    pub fn relay(&mut self, i: impl Into<RelayConfig>) -> &mut Self {
        self.relay = i.into();
        self
    }
    pub fn autonat(&mut self, i: impl Into<AutoNatConfig>) -> &mut Self {
        self.autonat = i.into();
        self
    }
    pub fn channel_size(&mut self, i: impl Into<usize>) -> &mut Self {
        self.channel_size = i.into();
        self
    }

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

        let mut swarm = SwarmBuilder::with_tokio_executor(
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
                    Either::Right((Some(_), _)) => {
                        // process swarm event
                    }
                }
            }
            Result::<(), anyhow::Error>::Ok(())
        });
        Ok(r)
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::p2p::transport::MemoryConfig;
    use libp2p::build_multiaddr;

    #[test]
    async fn basic_test() {
        let addr = build_multiaddr!(Memory(0));

        let relay = RelayConfig::default()
            .launch(Keypair::generate_ed25519(), MemoryConfig)
            .await
            .unwrap();

        relay.listen_on([addr.clone()]).await.unwrap();
        let listened = relay.get_addresses().await.unwrap();

        assert_eq!(listened, vec![addr]);
    }
}
