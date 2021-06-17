use crate::auth::{Action, AuthorizationPolicy, AuthorizationToken};
use anyhow::Result;
use ipfs_embed::Cid;
use serde::{Deserialize, Serialize};
use ssi::zcap::{Delegation, Invocation};

#[derive(Clone, Deserialize, Serialize)]
pub enum ZCAPAction {
    Read,
    Write,
}
#[derive(Clone)]
pub struct ZCAPInvocation(pub Invocation<ZCAPAction>);

impl AuthorizationToken for ZCAPInvocation {
    fn extract(auth_data: &str) -> Result<Self> {
        Ok(ZCAPInvocation(serde_json::from_str(auth_data)?))
    }

    fn action(&self) -> Action {
        match &self.0.capability_action {
            Some(a) => match a {
                ZCAPAction::Read => Action::Create {
                    orbit_id: Cid::default(),
                    parameters: "".to_string(),
                    content: vec![],
                },
                ZCAPAction::Write => Action::Create {
                    orbit_id: Cid::default(),
                    parameters: "".to_string(),
                    content: vec![],
                },
            },
            None => Action::Create {
                orbit_id: Cid::default(),
                parameters: "".to_string(),
                content: vec![],
            },
        }
    }
}

#[derive(Clone)]
pub struct ZCAPDelegation(pub Delegation<ZCAPAction, ()>);

#[rocket::async_trait]
impl AuthorizationPolicy for ZCAPDelegation {
    type Token = ZCAPInvocation;

    async fn authorize<'a>(&self, auth_token: &'a Self::Token) -> Result<()> {
        Ok(())
    }
}
