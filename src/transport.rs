use std::{
    collections::HashSet,
    future::{ready, Ready},
};

use ipfs::PeerId;
use libp2p::{core::Endpoint, noise::NoiseError};

pub fn auth_mapper<A, I>(
    i: I,
) -> impl Clone + FnOnce(((PeerId, A), Endpoint)) -> Ready<Result<(PeerId, A), AuthError>>
where
    I: IntoIterator<Item = PeerId>,
{
    let peer_list: HashSet<PeerId> = i.into_iter().collect();
    move |((peer_id, a), endpoint): ((PeerId, A), Endpoint)| {
        ready({
            match endpoint {
                Endpoint::Dialer => Ok((peer_id, a)),
                Endpoint::Listener => {
                    if peer_list.contains(&peer_id) {
                        Ok((peer_id, a))
                    } else {
                        Err(AuthError::NotInList)
                    }
                }
            }
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("`{0}`")]
    Noise(#[from] NoiseError),
    #[error("Unauthorised: peer attempting connection is not in approved peer list.")]
    NotInList,
}

#[cfg(test)]
mod test {
    use std::convert::TryFrom;

    use crate::{ipfs::create_ipfs, relay::test::test_relay};
    use ipfs::{MultiaddrWithoutPeerId, Protocol};
    use libp2p::identity::Keypair;
    use tempdir::TempDir;

    #[tokio::test]
    async fn authorised() -> anyhow::Result<()> {
        let id = "transport_unauthorized".to_string();
        let temp_dir = TempDir::new(&id)?;
        let relay = test_relay().await?;

        let alice_keypair = Keypair::generate_ed25519();
        let alice_peer_id = alice_keypair.public().into_peer_id();
        let alice_path = temp_dir.path().join("alice");
        let bob_keypair = Keypair::generate_ed25519();
        let bob_peer_id = bob_keypair.public().into_peer_id();
        let bob_path = temp_dir.path().join("bob");

        let (alice, ipfs_task, _alice_behaviour_process) =
            create_ipfs(id.clone(), &alice_path, alice_keypair, vec![bob_peer_id]).await?;
        tokio::task::spawn(ipfs_task);
        let (bob, ipfs_task, _bob_behaviour_process) =
            create_ipfs(id.clone(), &bob_path, bob_keypair, vec![]).await?;
        tokio::task::spawn(ipfs_task);

        alice
            .connect(MultiaddrWithoutPeerId::try_from(relay.internal())?.with(relay.id.clone()))
            .await?;

        bob.connect(
            MultiaddrWithoutPeerId::try_from(
                relay
                    .external()
                    .with(Protocol::P2p(relay.id.clone().into()))
                    .with(Protocol::P2pCircuit),
            )?
            .with(alice_peer_id),
        )
        .await
        .expect("authorised peer (bob) could not connect to alice");

        Ok(())
    }

    #[tokio::test]
    async fn unauthorised() -> anyhow::Result<()> {
        let id = "transport_unauthorized".to_string();
        let temp_dir = TempDir::new(&id)?;
        let relay = test_relay().await?;

        let alice_keypair = Keypair::generate_ed25519();
        let alice_peer_id = alice_keypair.public().into_peer_id();
        let alice_path = temp_dir.path().join("alice");
        let bob_keypair = Keypair::generate_ed25519();
        let _bob_peer_id = bob_keypair.public().into_peer_id();
        let bob_path = temp_dir.path().join("bob");

        let (alice, ipfs_task, _alice_behaviour_process) =
            create_ipfs(id.clone(), &alice_path, alice_keypair, vec![]).await?;
        tokio::task::spawn(ipfs_task);
        let (bob, ipfs_task, _bob_behaviour_process) =
            create_ipfs(id.clone(), &bob_path, bob_keypair, vec![]).await?;
        tokio::task::spawn(ipfs_task);

        alice
            .connect(MultiaddrWithoutPeerId::try_from(relay.internal())?.with(relay.id.clone()))
            .await?;

        bob.connect(
            MultiaddrWithoutPeerId::try_from(
                relay
                    .external()
                    .with(Protocol::P2p(relay.id.into()))
                    .with(Protocol::P2pCircuit),
            )?
            .with(alice_peer_id),
        )
        .await
        .expect_err("unauthorised peer (bob) connected to alice");

        Ok(())
    }
}
