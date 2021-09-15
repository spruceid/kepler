use anyhow::Result;
use libp2p::{
    core::{
        identity,
        multiaddr::Protocol,
        upgrade::{self, SelectUpgrade},
        Multiaddr, PeerId, Transport,
    },
    noise::{self, Keypair, NoiseConfig, X25519Spec},
    relay::{new_transport_and_behaviour, Relay, RelayConfig, RelayTransport},
    swarm::Swarm,
    tcp::{tokio::Tcp, GenTcpConfig, TokioTcpConfig as TcpConfig},
    yamux::YamuxConfig,
};
use std::str::FromStr;

pub fn open_relay(
    addr: String,
    port: u64,
    // ) -> Result<(RelayTransport<GenTcpConfig<Tcp>>, Relay, Multiaddr)> {
) -> Result<Multiaddr> {
    let local_key = identity::Keypair::generate_ed25519();
    let local_public_key = local_key.public();
    let local_peer_id = local_public_key.into_peer_id();

    let transport = TcpConfig::new().nodelay(true).port_reuse(true);
    let (relay_transport, relay_behaviour) =
        new_transport_and_behaviour(RelayConfig::default(), transport);

    let dh_key = noise::Keypair::<X25519Spec>::new()
        .into_authentic(&local_key)
        .unwrap();
    let transport = relay_transport
        .upgrade(upgrade::Version::V1)
        .authenticate(NoiseConfig::xx(dh_key).into_authenticated())
        .multiplex(YamuxConfig::default())
        .boxed();

    let mut swarm = Swarm::new(transport, relay_behaviour, local_peer_id);

    let relay_addr = format!("/ip4/{}/tcp/{}", addr, port).parse::<Multiaddr>()?;

    // // Listen for incoming connections via relay node (1234).
    swarm.listen_on(relay_addr.clone()).unwrap();

    // // Dial node (5678) via relay node (1234).
    // let dst_addr = relay_addr.clone().with(Protocol::Memory(5678));
    // swarm.dial_addr(dst_addr).unwrap();

    Ok(relay_addr
        .with(Protocol::P2p(local_peer_id.into()))
        .with(Protocol::P2pCircuit))
}
