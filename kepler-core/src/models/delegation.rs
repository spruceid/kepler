use super::super::{events::Delegation, models::*, util};
use crate::hash::Hash;
use kepler_lib::{authorization::KeplerDelegation, resolver::DID_METHODS};
use sea_orm::{entity::prelude::*, sea_query::Condition, ConnectionTrait};
use time::OffsetDateTime;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "delegation")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: Vec<u8>,
    #[sea_orm(primary_key)]
    pub orbit: String,

    pub seq: u64,
    pub epoch_id: Vec<u8>,
    pub epoch_seq: u64,

    pub delegator: String,
    pub expiry: Option<OffsetDateTime>,
    pub issued_at: Option<OffsetDateTime>,
    pub not_before: Option<OffsetDateTime>,
    pub serialization: Vec<u8>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    // inverse relation, delegations belong to delegators
    #[sea_orm(
        belongs_to = "actor::Entity",
        from = "(Column::Delegator, Column::Orbit)",
        to = "(actor::Column::Id, actor::Column::Orbit)"
    )]
    Delegator,
    #[sea_orm(has_one = "actor::Entity")]
    Delegatee,
    // inverse relation, delegations belong to epochs
    #[sea_orm(
        belongs_to = "epoch::Entity",
        from = "(Column::EpochId, Column::Orbit)",
        to = "(epoch::Column::Id, epoch::Column::Orbit)"
    )]
    Epoch,
    #[sea_orm(has_many = "invocation::Entity")]
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
    #[error("Unauthorized Delegator: {0}")]
    UnauthorizedDelegator(String),
    #[error("Unauthorized Capability: {0}/{1}")]
    UnauthorizedCapability(String, String),
    #[error("Cannot find parent delegation")]
    MissingParents,
}

pub async fn process<C: ConnectionTrait>(
    root: &str,
    orbit: &str,
    db: &C,
    delegation: Delegation,
    seq: u64,
    epoch: Hash,
    epoch_seq: u64,
) -> Result<Hash, Error> {
    let Delegation(d, ser) = delegation;
    verify(&d).await?;

    let d_info = util::DelegationInfo::try_from(d).map_err(DelegationError::ParameterExtraction)?;
    validate(db, root, orbit, &d_info).await?;

    Ok(save(db, orbit, d_info, ser, seq, epoch, epoch_seq)
        .await?
        .into())
}

// verify signatures and time
async fn verify(delegation: &KeplerDelegation) -> Result<(), Error> {
    match delegation {
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
    Ok(())
}

// verify parenthood and authorization
async fn validate<C: ConnectionTrait>(
    db: &C,
    root: &str,
    orbit: &str,
    delegation: &util::DelegationInfo,
) -> Result<(), Error> {
    if !delegation.parents.is_empty() || !delegation.delegator.starts_with(root) {
        let parents = Entity::find()
            .filter(Column::Orbit.eq(orbit))
            .filter(delegation.parents.iter().fold(Condition::any(), |cond, p| {
                cond.add(Column::Id.eq(p.hash().to_bytes()))
            }))
            .all(db)
            .await?;
        if parents.len() != delegation.parents.len() {
            return Err(DelegationError::MissingParents)?;
        };

        let mut parent_abilities = Vec::new();
        for parent in parents {
            // get delegatee of parent
            let delegatee = parent
                .find_related(actor::Entity)
                .one(db)
                .await?
                .ok_or_else(|| DelegationError::MissingParents)?;
            // check parent's delegatee is delegator of this one
            if delegatee.id != delegation.delegator {
                return Err(DelegationError::UnauthorizedDelegator(
                    delegation.delegator.clone(),
                ))?;
            };
            // check expiry of parent is not before this one
            if parent.expiry < delegation.expiry {
                return Err(DelegationError::InvalidTime)?;
            };
            // parent nbf must come before child nbf, child nbf must exist if parent nbf does
            if parent
                .not_before
                .map(|pnbf| delegation.not_before.map(|nbf| pnbf > nbf).unwrap_or(true))
                .unwrap_or(false)
            {
                return Err(DelegationError::InvalidTime)?;
            };
            parent_abilities.extend(parent.find_related(abilities::Entity).all(db).await?);
        }
        for ab in delegation.capabilities.iter() {
            if !parent_abilities
                .iter()
                .any(|pab| ab.resource.starts_with(&pab.resource) && ab.action == pab.ability)
            {
                return Err(DelegationError::UnauthorizedCapability(
                    ab.resource.clone(),
                    ab.action.clone(),
                ))?;
            }
        }
    };
    Ok(())
}

async fn save<C: ConnectionTrait>(
    db: &C,
    orbit: &str,
    delegation: util::DelegationInfo,
    serialization: Vec<u8>,
    seq: u64,
    epoch: Hash,
    epoch_seq: u64,
) -> Result<Hash, Error> {
    // save delegatee actor
    // no need to save delegator, should already exist
    actor::ActiveModel::from(actor::Model {
        id: delegation.delegate,
        orbit: orbit.to_string(),
    })
    .save(db)
    .await?;

    let hash: Hash = crate::hash::hash(&serialization);

    // save delegation
    ActiveModel::from(Model {
        seq,
        epoch_id: epoch.into(),
        epoch_seq,
        id: hash.clone().into(),
        delegator: delegation.delegator,
        expiry: delegation.expiry,
        issued_at: delegation.issued_at,
        not_before: delegation.not_before,
        serialization,
        orbit: orbit.to_string(),
    })
    .save(db)
    .await?;

    // save abilities
    for ab in delegation.capabilities {
        abilities::ActiveModel::from(abilities::Model {
            delegation: hash.clone().into(),
            resource: ab.resource,
            ability: ab.action,
            caveats: None,
            orbit: orbit.to_string(),
        })
        .save(db)
        .await?;
    }

    Ok(hash)
}
