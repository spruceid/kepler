use crate::p2p::{
    transport::{build_transport, IntoTransport},
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
    core::{identity::Keypair, Multiaddr, Transport},
    identify::Behaviour as Identify,
    identity::PublicKey,
    noise,
    ping::{Behaviour as Ping, Config as PingConfig},
    relay::{Behaviour as Relay, Config as RelayConfig},
    swarm::{NetworkBehaviour, Swarm, SwarmBuilder},
    PeerId,
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

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("failed to listen on multiaddress: {0}")]
    Listen(Multiaddr),
    #[error("failed to dial multiaddress: {0}")]
    Dial(Multiaddr),
    #[error("failed to send message: {0}")]
    SendError(#[from] mpsc::SendError),
    #[error("failed to recieve behaviour response: {0}")]
    RecieveError(#[from] oneshot::Canceled),
    #[error("failed to open listener: {0}")]
    TransportError(#[from] libp2p::TransportError<std::io::Error>),
}

#[derive(Debug)]
enum Message {
    GetAddresses(oneshot::Sender<Vec<Multiaddr>>),
    ListenOn(Vec<Multiaddr>, oneshot::Sender<Result<(), Error>>),
    Dial(Multiaddr, oneshot::Sender<Result<(), Error>>),
    GetConnectedPeers(oneshot::Sender<Vec<PeerId>>),
}

impl RelayNode {
    pub fn id(&self) -> &PeerId {
        &self.id
    }
    pub async fn get_addresses(&mut self) -> Result<Vec<Multiaddr>, Error> {
        let (s, r) = oneshot::channel();
        self.sender.send(Message::GetAddresses(s)).await?;
        Ok(r.await?)
    }

    pub async fn listen_on(
        &mut self,
        addr: impl IntoIterator<Item = Multiaddr>,
    ) -> Result<(), Error> {
        let (s, r) = oneshot::channel();
        self.sender
            .send(Message::ListenOn(addr.into_iter().collect(), s))
            .await?;
        r.await?
    }

    pub async fn dial(&mut self, addr: Multiaddr) -> Result<(), Error> {
        let (s, r) = oneshot::channel();
        self.sender.send(Message::Dial(addr, s)).await?;
        r.await?
    }

    pub async fn connected_peers(&mut self) -> Result<Vec<PeerId>, Error> {
        let (s, r) = oneshot::channel();
        self.sender.send(Message::GetConnectedPeers(s)).await?;
        Ok(r.await?)
    }
}

#[derive(Debug)]
pub struct Config {
    identify: IdentifyConfig,
    ping: PingConfig,
    relay: RelayConfig,
    autonat: AutoNatConfig,
    channel_size: usize,
    transport_timeout: std::time::Duration,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            identify: IdentifyConfig::default(),
            ping: PingConfig::default(),
            relay: RelayConfig::default(),
            autonat: AutoNatConfig::default(),
            channel_size: 100,
            transport_timeout: std::time::Duration::from_secs(20),
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
    pub fn transport_timeout(&mut self, i: impl Into<std::time::Duration>) -> &mut Self {
        self.transport_timeout = i.into();
        self
    }

    fn build(self, pubkey: PublicKey) -> Behaviour {
        let peer_id = pubkey.to_peer_id();
        Behaviour {
            identify: Identify::new(self.identify.into_config(pubkey)),
            ping: Ping::new(self.ping),
            relay: Relay::new(peer_id, self.relay),
            autonat: AutoNat::new(peer_id, self.autonat),
        }
    }

    pub fn launch<T>(self, keypair: Keypair, transport: T) -> Result<RelayNode, BuildError<T>>
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
        let (sender, reciever) = mpsc::channel(self.channel_size);

        let swarm = SwarmBuilder::with_tokio_executor(
            build_transport(
                transport
                    .into_transport()
                    .map_err(BuildError::TransportConfig)?,
                self.transport_timeout,
                &keypair,
            )?,
            self.build(local_public_key),
            id,
        )
        .build();

        tokio::spawn(poll_swarm(swarm, reciever));

        Ok(RelayNode { id, sender })
    }
}

#[derive(thiserror::Error, Debug)]
pub enum BuildError<T>
where
    T: IntoTransport,
{
    #[error(transparent)]
    TransportConfig(T::Error),
    #[error(transparent)]
    Noise(#[from] noise::NoiseError),
}

#[derive(thiserror::Error, Debug)]
pub enum SwarmError {
    #[error("failed to send response via oneshot")]
    SendError,
    #[error("failed to dial multiaddress: {0}")]
    DialError(#[from] libp2p::swarm::DialError),
}

async fn poll_swarm(
    mut swarm: Swarm<Behaviour>,
    mut reciever: mpsc::Receiver<Message>,
) -> Result<(), SwarmError> {
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
                        Ok(_) => s.send(Ok(())).map_err(|_| SwarmError::SendError)?,
                        // if one fails, roll back all of them
                        Err((e, listeners)) => {
                            for l in listeners {
                                swarm.remove_listener(l);
                            }
                            s.send(Err(e.into())).map_err(|_| SwarmError::SendError)?
                        }
                    };
                }
                Message::GetAddresses(s) => s
                    .send(swarm.listeners().cloned().collect())
                    .map_err(|_| SwarmError::SendError)?,
                Message::Dial(addr, s) => {
                    swarm.dial(addr)?;
                    s.send(Ok(())).map_err(|_| SwarmError::SendError)?
                }
                Message::GetConnectedPeers(s) => s
                    .send(swarm.connected_peers().map(|p| p.to_owned()).collect())
                    .map_err(|_| SwarmError::SendError)?,
            },
            Either::Right((Some(_), _)) => {
                // process swarm event
            }
        }
    }
    Result::Ok(())
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::p2p::transport::{Both, MemoryConfig, TcpConfig};
    use libp2p::build_multiaddr;

    #[test]
    async fn basic_test() {
        let addr = build_multiaddr!(Memory(1u8));

        let mut relay = Config::default()
            .launch(Keypair::generate_ed25519(), MemoryConfig)
            .unwrap();

        relay.listen_on([addr.clone()]).await.unwrap();
        let listened = relay.get_addresses().await.unwrap();

        assert_eq!(listened, vec![addr]);
    }
}
