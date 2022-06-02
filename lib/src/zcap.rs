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

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct InvProps {
    pub invocation_target: ResourceId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub valid_from: Option<String>,
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
            expires: None,
            valid_from: None,
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
    let mut capability_chain = delegation
        .proof
        .as_ref()
        .and_then(|proof| proof.property_set.as_ref())
        .and_then(|props| props.get("capabilityChain"))
        .and_then(|chain| chain.as_array().cloned())
        .unwrap_or_default();
    capability_chain
        .push(serde_json::to_value(&delegation).map_err(Error::DelegationToJsonValueConversion)?);

    let mut proof_props = std::collections::HashMap::<String, Value>::new();
    proof_props.insert(
        "capabilityChain".into(),
        serde_json::to_value(capability_chain).map_err(Error::DelegationToJsonValueConversion)?,
    );

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
