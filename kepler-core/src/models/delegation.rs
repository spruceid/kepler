use super::super::{events::Delegation, models::*, util};
use kepler_lib::{
    authorization::KeplerDelegation,
    resolver::DID_METHODS,
    resource::{KRIParseError, ResourceId},
};
use sea_orm::{entity::prelude::*, sea_query::Condition, ConnectionTrait};
use time::OffsetDateTime;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "delegation")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Vec<u8>,
    pub delegator: String,
    pub expiry: Option<OffsetDateTime>,
    pub issued_at: Option<OffsetDateTime>,
    pub not_before: Option<OffsetDateTime>,
    pub serialized: Vec<u8>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    // inverse relation, delegations belong to delegators
    #[sea_orm(
        belongs_to = "actor::Entity",
        from = "Column::Delegator",
        to = "super::actor::Column::Id"
    )]
    Delegator,
    #[sea_orm(has_one = "actor::Entity")]
    Delegatee,
    // inverse relation, delegations belong to epochs
    #[sea_orm(
        belongs_to = "epoch::Entity",
        from = "Column::Id",
        to = "epoch::Column::Id"
    )]
    Epoch,
    #[sea_orm(has_many = "super::invocation::Entity")]
    Invocation,
    #[sea_orm(has_many = "revocation::Entity")]
    Revocation,
    #[sea_orm(has_many = "abilities::Entity")]
    Abilities,
}

impl Related<actor::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Delegatee.def()
    }
}

impl Related<epoch::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Epoch.def()
    }
}

impl Related<invocation::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Invocation.def()
    }
}

impl Related<revocation::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Revocation.def()
    }
}

impl Related<abilities::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Abilities.def()
    }
}

#[derive(Copy, Clone, Debug)]
pub struct ParentToChild;

impl Linked for ParentToChild {
    type FromEntity = Entity;

    type ToEntity = Entity;

    fn link(&self) -> Vec<RelationDef> {
        use super::super::relationships::parent_delegations;
        vec![
            parent_delegations::Relation::Parent.def().rev(),
            parent_delegations::Relation::Child.def(),
        ]
    }
}

#[derive(Copy, Clone, Debug)]
pub struct ChildToParent;

impl Linked for ChildToParent {
    type FromEntity = Entity;

    type ToEntity = Entity;

    fn link(&self) -> Vec<RelationDef> {
        use super::super::relationships::parent_delegations;
        vec![
            parent_delegations::Relation::Child.def().rev(),
            parent_delegations::Relation::Parent.def(),
        ]
    }
}

impl ActiveModelBehavior for ActiveModel {}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Db(#[from] DbErr),
    #[error(transparent)]
    InvalidDelegation(#[from] DelegationError),
}

#[derive(Debug, thiserror::Error)]
pub enum DelegationError {
    #[error(transparent)]
    ParameterExtraction(#[from] util::DelegationError),
    #[error("Delegation expired or not yet valid")]
    InvalidTime,
    #[error("Failed to verify signature")]
    InvalidSignature,
    #[error("Unauthorized Delegator")]
    UnauthorizedDelegator,
    #[error("Unauthorized Capability")]
    UnauthorizedCapability,
    #[error("Cannot find parent delegation")]
    MissingParents,
}

pub async fn process<C: ConnectionTrait>(
    root: &str,
    db: &C,
    delegation: Delegation,
) -> Result<[u8; 32], Error> {
    let Delegation(d, ser) = delegation;
    // verify signatures and time
    match d {
        KeplerDelegation::Ucan(ref ucan) => {
            ucan.verify_signature(DID_METHODS.to_resolver())
                .await
                .map_err(|_| DelegationError::InvalidSignature)?;
            ucan.payload
                .validate_time(None)
                .map_err(|_| DelegationError::InvalidTime)?;
        }
        KeplerDelegation::Cacao(ref cacao) => {
            cacao
                .verify()
                .await
                .map_err(|_| DelegationError::InvalidSignature)?;
            if !cacao.payload().valid_now() {
                return Err(DelegationError::InvalidTime)?;
            }
        }
    };
    let d_info = util::DelegationInfo::try_from(d).map_err(DelegationError::ParameterExtraction)?;
    if !d_info.parents.is_empty() || !d_info.delegator.starts_with(root) {
        let parents = Entity::find()
            .filter(d_info.parents.iter().fold(Condition::any(), |cond, p| {
                cond.add(Column::Id.eq(p.to_bytes()))
            }))
            .all(db)
            .await?;
        let mut parent_abilities = Vec::new();
        for parent in parents {
            // get delegatee of parent
            let delegatee = parent
                .find_related(actor::Entity)
                .one(db)
                .await?
                .ok_or_else(|| DelegationError::MissingParents)?;
            // check parent's delegatee is delegator of this one
            if delegatee.id != d_info.delegator {
                return Err(DelegationError::UnauthorizedDelegator)?;
            };
            // check expiry of parent is not before this one
            if parent.expiry < d_info.expiry {
                return Err(DelegationError::InvalidTime)?;
            };
            // parent nbf must come before child nbf, child nbf must exist if parent nbf does
            if parent
                .not_before
                .map(|pnbf| d_info.not_before.map(|nbf| pnbf > nbf).unwrap_or(true))
                .unwrap_or(false)
            {
                return Err(DelegationError::InvalidTime)?;
            };
            parent_abilities.extend(parent.find_related(abilities::Entity).all(db).await?);
        }
        for ab in d_info.capabilities.iter() {
            if !parent_abilities
                .iter()
                .any(|pab| ab.resource.starts_with(&pab.resource) && ab.action == pab.action)
            {
                return Err(DelegationError::UnauthorizedCapability)?;
            }
        }
    };

    // save delegator actor
    actor::ActiveModel::from(actor::Model {
        id: d_info.delegator.clone(),
    })
    .save(db)
    .await?;

    // save delegatee actor
    actor::ActiveModel::from(actor::Model {
        id: d_info.delegate,
    })
    .save(db)
    .await?;

    let hash: [u8; 32] = blake3::hash(&ser).into();

    // save delegation
    ActiveModel::from(Model {
        id: hash.clone().into(),
        delegator: d_info.delegator,
        expiry: d_info.expiry,
        issued_at: d_info.issued_at,
        not_before: d_info.not_before,
        serialized: ser,
    })
    .save(db)
    .await?;

    for ab in d_info.capabilities {
        // save ability
        abilities::ActiveModel::from(abilities::Model {
            delegation: hash.clone().into(),
            resource: ab.resource,
            action_namespace: "".to_string(),
            action: ab.action,
            caveats: None,
        })
        .save(db)
        .await?;
    }

    Ok(hash)
}
