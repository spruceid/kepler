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
    #[error("Unauthorized: peer attempting connection is not in approved peer list.")]
    NotInList,
}
