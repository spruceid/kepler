use anyhow::Result;
use libp2p::{
    core::{identity, multiaddr::Protocol, upgrade, Multiaddr, Transport},
    noise::{self, NoiseConfig, X25519Spec},
    relay::{new_transport_and_behaviour, RelayConfig},
    swarm::Swarm,
    tcp::TokioTcpConfig as TcpConfig,
    yamux::YamuxConfig,
};

pub fn open_relay(addr: String, port: u64) -> Result<Multiaddr> {
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

    let relay_addr = format!("/ip4/{}/tcp/{}", addr, port).parse::<Multiaddr>()?;
    let mut swarm = Swarm::new(transport, relay_behaviour, local_peer_id);
    swarm.listen_on(relay_addr.clone()).unwrap();

    Ok(relay_addr
        // TODO does it work without specifying the relay's peer id?
        .with(Protocol::P2p(local_peer_id.into()))
        .with(Protocol::P2pCircuit))
}
