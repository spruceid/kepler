use kepler_lib::resource::OrbitId;
use kepler_lib::ssi::{
    did::{Document, RelativeDIDURL, Service, VerificationMethod, DIDURL},
    did_resolve::DIDResolver,
    one_or_many::OneOrMany,
};
use libp2p::{Multiaddr, PeerId};
use std::{convert::TryFrom, str::FromStr};
use thiserror::Error;

/// An implementation of an Orbit Manifest.
///
/// Orbit Manifests are [DID Documents](https://www.w3.org/TR/did-spec-registries/#did-methods) used directly as the root of a capabilities
/// authorization framework. This enables Orbits to be managed using independant DID lifecycle management tools.
#[derive(Clone, Debug)]
pub struct Manifest {
    id: OrbitId,
    delegators: Vec<DIDURL>,
    invokers: Vec<DIDURL>,
    bootstrap_peers: BootstrapPeers,
}

impl Manifest {
    /// ID of the Orbit, usually a DID
    pub fn id(&self) -> &OrbitId {
        &self.id
    }

    /// The set of Peers discoverable from the Orbit Manifest.
    pub fn bootstrap_peers(&self) -> &BootstrapPeers {
        &self.bootstrap_peers
    }

    /// The set of [Verification Methods](https://www.w3.org/TR/did-core/#verification-methods) who are authorized to delegate any capability.
    pub fn delegators(&self) -> &[DIDURL] {
        &self.delegators
    }

    /// The set of [Verification Methods](https://www.w3.org/TR/did-core/#verification-methods) who are authorized to invoke any capability.
    pub fn invokers(&self) -> &[DIDURL] {
        &self.invokers
    }

    pub async fn resolve_dyn(
        id: &OrbitId,
        resolver: Option<&dyn DIDResolver>,
    ) -> Result<Option<Self>, ResolutionError> {
        resolve_dyn(id, resolver).await
    }

    pub async fn resolve<D: DIDResolver>(
        id: &OrbitId,
        resolver: &D,
    ) -> Result<Option<Self>, ResolutionError> {
        resolve(id, resolver).await
    }
}

#[derive(Clone, Debug, Hash)]
pub struct BootstrapPeers {
    pub id: String,
    pub peers: Vec<BootstrapPeer>,
}

#[derive(Clone, Debug, Hash)]
pub struct BootstrapPeer {
    pub id: PeerId,
    pub addrs: Vec<Multiaddr>,
}

impl<'a> From<(Document, &'a str)> for Manifest {
    fn from((d, n): (Document, &'a str)) -> Self {
        let bootstrap_peers = d
            .select_service(n)
            .and_then(|s| BootstrapPeers::try_from(s).ok())
            .unwrap_or_else(|| BootstrapPeers {
                id: n.into(),
                peers: vec![],
            });
        let Document {
            id,
            capability_delegation,
            capability_invocation,
            verification_method,
            ..
        } = d;
        Self {
            delegators: capability_delegation
                .or_else(|| verification_method.clone())
                .unwrap_or_default()
                .into_iter()
                .map(|vm| id_from_vm(&id, vm))
                .collect(),
            invokers: capability_invocation
                .or_else(|| verification_method.clone())
                .unwrap_or_default()
                .into_iter()
                .map(|vm| id_from_vm(&id, vm))
                .collect(),
            bootstrap_peers,
            id: OrbitId::new(
                id.split_once(':').map(|(_, s)| s.into()).unwrap_or(id),
                n.into(),
            ),
        }
    }
}

#[derive(Error, Debug)]
pub enum ResolutionError {
    #[error("DID Resolution Error: {0}")]
    Resolver(String),
    #[error("DID Deactivated")]
    Deactivated,
}

pub async fn resolve_dyn(
    id: &OrbitId,
    resolver: Option<&dyn DIDResolver>,
) -> Result<Option<Manifest>, ResolutionError> {
    let (md, doc, doc_md) = resolver
        .unwrap_or_else(|| kepler_lib::resolver::DID_METHODS.to_resolver())
        .resolve(&id.did(), &Default::default())
        .await;

    match (md.error, doc, doc_md.and_then(|d| d.deactivated)) {
        (Some(e), _, _) => Err(ResolutionError::Resolver(e)),
        (_, _, Some(true)) => Err(ResolutionError::Deactivated),
        (_, None, _) => Ok(None),
        (None, Some(d), None | Some(false)) => Ok(Some((d, id.name()).into())),
    }
}

pub async fn resolve<D: DIDResolver>(
    id: &OrbitId,
    resolver: &D,
) -> Result<Option<Manifest>, ResolutionError> {
    let (md, doc, doc_md) = resolver.resolve(&id.did(), &Default::default()).await;

    match (md.error, doc, doc_md.and_then(|d| d.deactivated)) {
        (Some(e), _, _) => Err(ResolutionError::Resolver(e)),
        (_, _, Some(true)) => Err(ResolutionError::Deactivated),
        (_, None, _) => Ok(None),
        (None, Some(d), None | Some(false)) => Ok(Some((d, id.name()).into())),
    }
}

#[derive(Error, Debug)]
pub enum ServicePeersConversionError {
    #[error(transparent)]
    IdParse(<PeerId as FromStr>::Err),
    #[error("Missing KeplerOrbitPeer type string")]
    WrongType,
}

impl TryFrom<&Service> for BootstrapPeers {
    type Error = ServicePeersConversionError;
    fn try_from(s: &Service) -> Result<Self, Self::Error> {
        if s.type_.any(|t| t == "KeplerOrbitPeers") {
            Ok(Self {
                id: s
                    .id
                    .rsplit_once('#')
                    .map(|(_, id)| id)
                    .unwrap_or_else(|| &s.id)
                    .into(),
                peers: s
                    .service_endpoint
                    .as_ref()
                    .unwrap_or(&OneOrMany::Many(vec![]))
                    .into_iter()
                    // TODO parse peers from objects or multiaddrs
                    .filter_map(|_| None)
                    .collect(),
            })
        } else {
            Err(Self::Error::WrongType)
        }
    }
}

fn id_from_vm(did: &str, vm: VerificationMethod) -> DIDURL {
    match vm {
        VerificationMethod::DIDURL(d) => d,
        VerificationMethod::RelativeDIDURL(f) => f.to_absolute(did),
        VerificationMethod::Map(m) => {
            if let Ok(abs_did_url) = DIDURL::from_str(&m.id) {
                abs_did_url
            } else if let Ok(rel_did_url) = RelativeDIDURL::from_str(&m.id) {
                rel_did_url.to_absolute(did)
            } else {
                // HACK well-behaved did methods should not allow id's which lead to this path
                DIDURL {
                    did: m.id,
                    ..Default::default()
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kepler_lib::resolver::DID_METHODS;
    use kepler_lib::ssi::{
        did::{Source, DIDURL},
        jwk::JWK,
    };
    use std::convert::TryInto;

    #[test]
    async fn basic_manifest() {
        let j = JWK::generate_secp256k1().unwrap();
        let did: DIDURL = DIDURL {
            did: DID_METHODS
                .generate(&Source::KeyAndPattern(&j, "pkh:tz"))
                .unwrap()
                .parse()
                .unwrap(),
            fragment: Some("default".to_string()),
            ..Default::default()
        };

        let _md = Manifest::resolve_dyn(&did.try_into().unwrap(), None)
            .await
            .unwrap()
            .unwrap();
    }
}
