use crate::resource::ResourceId;
use didkit::DID_METHODS;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ssi::{
    cacao_zcap::CacaoZcap2022Delegation,
    jwk::JWK,
    ldp::LinkedDataProofs,
    vc::{LinkedDataProofOptions, ProofPurpose, URI},
    zcap::{Contexts, Invocation},
};
use std::collections::HashMap;
use uuid::Uuid;

#[cfg(not(feature = "wasm"))]
pub use verify::*;

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct InvProps {
    pub invocation_target: ResourceId,
    #[serde(flatten)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra_fields: Option<HashMap<String, Value>>,
}

pub type KeplerDelegation = CacaoZcap2022Delegation;
pub type KeplerInvocation = Invocation<InvProps>;

pub async fn make_invocation(
    invocation_target: ResourceId,
    delegation: KeplerDelegation,
    jwk: &JWK,
    verification_method: String,
) -> Result<KeplerInvocation, Error> {
    let invocation = {
        let context = Contexts::default();
        let id = URI::String(format!("urn:uuid:{}", Uuid::new_v4()));
        let property_set = InvProps {
            invocation_target,
            extra_fields: None,
        };
        KeplerInvocation {
            context,
            id,
            property_set,
            proof: None,
        }
    };

    let ldp_options = LinkedDataProofOptions {
        verification_method: Some(URI::String(verification_method)),
        proof_purpose: Some(ProofPurpose::CapabilityInvocation),
        ..Default::default()
    };
    let resolver = DID_METHODS.to_resolver();
    let capability_chain = serde_json::to_value([
        serde_json::to_value(delegation).map_err(Error::DelegationToJsonValueConversion)?
    ])
    .map_err(Error::DelegationToJsonValueConversion)?;

    let mut proof_props = std::collections::HashMap::<String, Value>::new();
    proof_props.insert("capabilityChain".into(), capability_chain);

    let proof = LinkedDataProofs::sign(&invocation, &ldp_options, resolver, jwk, Some(proof_props))
        .await
        .map_err(Error::FailedToGenerateProof)
        .unwrap();

    Ok(invocation.set_proof(proof))
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("failed to convert Delegation to serde_json::Value: {0}")]
    DelegationToJsonValueConversion(serde_json::Error),
    #[error("failed to generate proof for invocation: {0}")]
    FailedToGenerateProof(ssi::error::Error),
}

#[cfg(not(feature = "wasm"))]
mod verify {
    use super::{KeplerDelegation, KeplerInvocation, DID_METHODS};
    use async_trait::async_trait;

    #[derive(thiserror::Error, Debug)]
    #[error("failed to verify zcap: {0}")]
    pub struct VerificationError(String);

    pub type VerificationResult = Result<(), VerificationError>;

    #[async_trait]
    pub trait Verifiable {
        async fn verify(&self) -> VerificationResult;
    }

    #[async_trait]
    impl Verifiable for KeplerDelegation {
        async fn verify(&self) -> VerificationResult {
            if let Some(e) = self
                .verify(Default::default(), DID_METHODS.to_resolver())
                .await
                .errors
                .into_iter()
                .next()
            {
                Err(VerificationError(e))
            } else {
                Ok(())
            }
        }
    }

    #[async_trait]
    impl Verifiable for KeplerInvocation {
        async fn verify(&self) -> VerificationResult {
            if let Some(e) = self
                .verify_signature(Default::default(), DID_METHODS.to_resolver())
                .await
                .errors
                .into_iter()
                .next()
            {
                Err(VerificationError(e))
            } else {
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use ssi::did::Source;
    use ssi::vc::get_verification_method;

    async fn make_invocation(
        invocation_target: ResourceId,
        jwk: &JWK,
        verification_method: String,
    ) -> Result<KeplerInvocation, Error> {
        let invocation = {
            let context = Contexts::default();
            let id = URI::String(format!("urn:uuid:{}", Uuid::new_v4()));
            let property_set = InvProps {
                invocation_target,
                extra_fields: None,
            };
            KeplerInvocation {
                context,
                id,
                property_set,
                proof: None,
            }
        };

        let ldp_options = LinkedDataProofOptions {
            verification_method: Some(URI::String(verification_method)),
            proof_purpose: Some(ProofPurpose::CapabilityInvocation),
            ..Default::default()
        };
        let resolver = DID_METHODS.to_resolver();

        let proof = LinkedDataProofs::sign(&invocation, &ldp_options, resolver, jwk, None)
            .await
            .map_err(Error::FailedToGenerateProof)
            .unwrap();

        Ok(invocation.set_proof(proof))
    }

    #[tokio::test]
    async fn test() {
        let invocation_target = "kepler:ens:example.eth://default/kv/#list".parse().unwrap();
        let jwk = JWK::generate_ed25519().unwrap();
        let did = DID_METHODS
            .generate(&Source::KeyAndPattern(&jwk, "key"))
            .unwrap();
        let did_resolver = DID_METHODS.to_resolver();
        let verification_method = get_verification_method(&did, did_resolver).await.unwrap();
        if let Some(e) = make_invocation(invocation_target, &jwk, verification_method)
            .await
            .unwrap()
            .verify_signature(Default::default(), did_resolver)
            .await
            .errors
            .first()
        {
            panic!("{}", e)
        }
    }
}
