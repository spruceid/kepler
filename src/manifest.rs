use libipld::cid::{multibase::Base, Cid, Error as CidError};
use libp2p::{Multiaddr, PeerId};

use crate::{auth::AuthorizationPolicy, orbit::AuthTokens};
use ssi::{
    did::{Document, RelativeDIDURL, Service, ServiceEndpoint, VerificationMethod, DIDURL},
    did_resolve::DIDResolver,
    one_or_many::OneOrMany,
};
use std::{
    convert::{TryFrom, TryInto},
    str::FromStr,
};
use thiserror::Error;

/// An implementation of an Orbit Manifest.
///
/// Orbit Manifests are [DID Documents](https://www.w3.org/TR/did-spec-registries/#did-methods) used directly as the root of a capabilities
/// authorization framework. This enables Orbits to be managed using independant DID lifecycle management tools.
#[derive(Clone, Debug)]
pub struct Manifest {
    id: String,
    delegators: Vec<DIDURL>,
    invokers: Vec<DIDURL>,
    bootstrap_peers: Vec<BootstrapPeer>,
}

impl Manifest {
    /// ID of the Orbit, usually a DID
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The set of Peers discoverable from the Orbit Manifest.
    pub fn bootstrap_peers(&self) -> &[BootstrapPeer] {
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

    /// Creates a Kepler URI for a given CID in the IPFS Store
    pub fn make_uri(&self, cid: &Cid) -> Result<String, CidError> {
        Ok(format!(
            "kepler:{}/ipfs/{}",
            self.id(),
            cid.to_string_of_base(Base::Base58Btc)?
        ))
    }

    pub async fn resolve_dyn(
        id: &str,
        resolver: Option<&dyn DIDResolver>,
    ) -> Result<Option<Self>, ResolutionError> {
        resolve_dyn(id, resolver).await
    }

    pub async fn resolve<D: DIDResolver>(
        id: &str,
        resolver: &D,
    ) -> Result<Option<Self>, ResolutionError> {
        resolve(id, resolver).await
    }
}

#[derive(Clone, Debug, Hash)]
pub struct BootstrapPeer {
    pub id: PeerId,
    pub addrs: Vec<Multiaddr>,
}

impl From<Document> for Manifest {
    fn from(
        Document {
            id,
            capability_delegation,
            capability_invocation,
            verification_method,
            service,
            ..
        }: Document,
    ) -> Self {
        Self {
            delegators: capability_delegation
                .or_else(|| verification_method.clone())
                .unwrap_or_else(|| vec![])
                .into_iter()
                .map(|vm| id_from_vm(&id, vm))
                .collect(),
            invokers: capability_invocation
                .or(verification_method)
                .unwrap_or_else(|| vec![])
                .into_iter()
                .map(|vm| id_from_vm(&id, vm))
                .collect(),
            bootstrap_peers: service
                .unwrap_or_else(|| vec![])
                .into_iter()
                .filter_map(|s| s.try_into().ok())
                .collect(),
            id,
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
    id: &str,
    resolver: Option<&dyn DIDResolver>,
) -> Result<Option<Manifest>, ResolutionError> {
    let (md, doc, doc_md) = resolver
        .unwrap_or(didkit::DID_METHODS.to_resolver())
        .resolve(id, &Default::default())
        .await;

    match (md.error, doc, doc_md.and_then(|d| d.deactivated)) {
        (Some(e), _, _) => Err(ResolutionError::Resolver(e)),
        (_, _, Some(true)) => Err(ResolutionError::Deactivated),
        (_, None, _) => Ok(None),
        (None, Some(d), None | Some(false)) => Ok(Some(d.into())),
    }
}

pub async fn resolve<D: DIDResolver>(
    id: &str,
    resolver: &D,
) -> Result<Option<Manifest>, ResolutionError> {
    let (md, doc, doc_md) = resolver.resolve(id, &Default::default()).await;

    match (md.error, doc, doc_md.and_then(|d| d.deactivated)) {
        (Some(e), _, _) => Err(ResolutionError::Resolver(e)),
        (_, _, Some(true)) => Err(ResolutionError::Deactivated),
        (_, None, _) => Ok(None),
        (None, Some(d), None | Some(false)) => Ok(Some(d.into())),
    }
}

#[derive(Error, Debug)]
pub enum ServicePeerConversionError {
    #[error(transparent)]
    IdParse(<PeerId as FromStr>::Err),
    #[error("Missing KeplerOrbitPeer type string")]
    WrongType,
}

impl TryFrom<Service> for BootstrapPeer {
    type Error = ServicePeerConversionError;
    fn try_from(s: Service) -> Result<Self, Self::Error> {
        if s.type_.any(|t| t == "KeplerOrbitPeer") {
            Ok(Self {
                id: s.id[1..].parse().map_err(|e| Self::Error::IdParse(e))?,
                addrs: s
                    .service_endpoint
                    .unwrap_or(OneOrMany::Many(vec![]))
                    .into_iter()
                    .filter_map(|e| match e {
                        ServiceEndpoint::URI(a) => {
                            a.strip_prefix("multiaddr:").and_then(|a| a.parse().ok())
                        }
                        _ => None,
                    })
                    .collect(),
            })
        } else {
            Err(Self::Error::WrongType)
        }
    }
}

impl From<BootstrapPeer> for Service {
    fn from(p: BootstrapPeer) -> Self {
        Self {
            id: format!("#{}", p.id),
            type_: OneOrMany::One("KeplerOrbitPeer".into()),
            service_endpoint: match p.addrs.len() {
                0 => None,
                1 => Some(OneOrMany::One(ServiceEndpoint::URI(format!(
                    "multiaddr:{}",
                    p.addrs[0]
                )))),
                _ => Some(OneOrMany::Many(
                    p.addrs
                        .into_iter()
                        .map(|a| ServiceEndpoint::URI(format!("multiaddr:{}", a)))
                        .collect(),
                )),
            },
            property_set: Default::default(),
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

#[rocket::async_trait]
impl AuthorizationPolicy<AuthTokens> for Manifest {
    async fn authorize(&self, auth_token: &AuthTokens) -> anyhow::Result<()> {
        match auth_token {
            AuthTokens::Tezos(token) => self.authorize(token).await,
            AuthTokens::ZCAP(token) => self.authorize(token.as_ref()).await,
            AuthTokens::SIWEDelegated(token) => self.authorize(token.as_ref()).await,
            AuthTokens::SIWEZcapDelegated(token) => self.authorize(token.as_ref()).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use didkit::DID_METHODS;
    use ssi::{did::Source, jwk::JWK};

    #[test]
    async fn basic_manifest() {
        let j = JWK::generate_secp256k1().unwrap();
        let did = DID_METHODS
            .generate(&Source::KeyAndPattern(&j, "pkh:tz"))
            .unwrap();

        let _md = Manifest::resolve_dyn(&did, None).await.unwrap().unwrap();
    }
}
