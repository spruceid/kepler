use super::super::{events::Delegation, models::*, relationships::*, util};
use crate::hash::Hash;
use crate::types::Resource;
use kepler_lib::{authorization::KeplerDelegation, resolver::DID_METHODS};
use sea_orm::{entity::prelude::*, ConnectionTrait};
use time::OffsetDateTime;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "delegation")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false, unique)]
    pub id: Hash,

    pub delegator: String,
    pub delegatee: String,
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
        from = "Column::Delegator",
        to = "actor::Column::Id"
    )]
    Delegator,
    #[sea_orm(
        belongs_to = "actor::Entity",
        from = "Column::Delegatee",
        to = "actor::Column::Id"
    )]
    Delegatee,
    // inverse relation, delegations belong to epochs
    #[sea_orm(
        belongs_to = "event_order::Entity",
        from = "Column::Id",
        to = "event_order::Column::Event"
    )]
    Ordering,
    #[sea_orm(has_many = "revocation::Entity")]
    Revocation,
    #[sea_orm(has_many = "abilities::Entity")]
    Abilities,
    #[sea_orm(
        belongs_to = "parent_delegations::Entity",
        from = "Column::Id",
        to = "parent_delegations::Column::Child"
    )]
    Parents,
}

impl Related<actor::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Delegator.def()
    }
}

impl Related<event_order::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Ordering.def()
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

impl Related<parent_delegations::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Parents.def()
    }
}

#[derive(Copy, Clone, Debug)]
pub struct ParentToChildren;

impl Linked for ParentToChildren {
    type FromEntity = Entity;

    type ToEntity = Entity;

    fn link(&self) -> Vec<RelationDef> {
        vec![
            parent_delegations::Relation::Parent.def().rev(),
            parent_delegations::Relation::Child.def(),
        ]
    }
}

#[derive(Copy, Clone, Debug)]
pub struct ChildToParents;

impl Linked for ChildToParents {
    type FromEntity = Entity;

    type ToEntity = Entity;

    fn link(&self) -> Vec<RelationDef> {
        vec![
            parent_delegations::Relation::Child.def().rev(),
            parent_delegations::Relation::Parent.def(),
        ]
    }
}

#[derive(Copy, Clone, Debug)]
pub struct Delegator;

impl Linked for Delegator {
    type FromEntity = Entity;

    type ToEntity = actor::Entity;

    fn link(&self) -> Vec<RelationDef> {
        vec![Relation::Delegator.def()]
    }
}

#[derive(Copy, Clone, Debug)]
pub struct Delegatee;

impl Linked for Delegatee {
    type FromEntity = Entity;

    type ToEntity = actor::Entity;

    fn link(&self) -> Vec<RelationDef> {
        vec![Relation::Delegatee.def()]
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
    #[error("Delegation expired or not yet valid")]
    InvalidTime,
    #[error("Failed to verify signature")]
    InvalidSignature,
    #[error("Unauthorized Delegator: {0}")]
    UnauthorizedDelegator(String),
    #[error("Unauthorized Capability: {0}, {1}")]
    UnauthorizedCapability(Resource, String),
    #[error("Cannot find parent delegation")]
    MissingParents,
}

pub(crate) async fn process<C: ConnectionTrait>(
    db: &C,
    delegation: Delegation,
) -> Result<Hash, Error> {
    let Delegation(d, ser) = delegation;
    verify(&d.delegation).await?;

    validate(db, &d).await?;

    save(db, d, ser).await
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
    delegation: &util::DelegationInfo,
) -> Result<(), Error> {
    // get caps which rely on delegated caps
    let dependant_caps: Vec<_> = delegation
        .capabilities
        .iter()
        .filter(|c| {
            // remove caps for which the delegator is the root authority
            c.resource
                .orbit()
                .map(|o| o.did() != delegation.delegator)
                .unwrap_or(true)
        })
        .collect();

    match (dependant_caps.is_empty(), delegation.parents.is_empty()) {
        // no dependant caps, no parents needed, must be valid
        (true, _) => Ok(()),
        // dependant caps, no parents, invalid
        (false, true) => Err(DelegationError::MissingParents.into()),
        // dependant caps, parents, check parents
        (false, false) => {
            // get parents which have
            let parents: Vec<_> = Entity::find()
                // the correct id
                .filter(Column::Id.is_in(delegation.parents.iter().map(|c| Hash::from(*c))))
                // the correct delegatee
                .filter(Column::Delegatee.eq(delegation.delegator.clone()))
                .all(db)
                .await?
                .into_iter()
                .filter(|p| {
                    // valid time bounds
                    p.expiry < delegation.expiry
                        && p.not_before
                            .map(|pnbf| delegation.not_before.map(|nbf| pnbf > nbf).unwrap_or(true))
                            .unwrap_or(false)
                })
                .collect();

            // get delegated abilities from each parent
            let parent_abilities = parents.load_many(abilities::Entity, db).await?;

            // check each dependant cap is supported by at least one parent cap
            match dependant_caps.iter().find(|c| {
                !parent_abilities
                    .iter()
                    .flatten()
                    .any(|pc| c.resource.extends(&pc.resource) && c.action == pc.ability)
            }) {
                Some(c) => Err(DelegationError::UnauthorizedCapability(
                    c.resource.clone(),
                    c.action.clone(),
                )
                .into()),
                None => Ok(()),
            }
        }
    }
}

async fn save<C: ConnectionTrait>(
    db: &C,
    delegation: util::DelegationInfo,
    serialization: Vec<u8>,
) -> Result<Hash, Error> {
    save_actor(delegation.delegate.clone(), db).await?;
    save_actor(delegation.delegator.clone(), db).await?;

    let hash: Hash = crate::hash::hash(&serialization);

    // save delegation
    Entity::insert(ActiveModel::from(Model {
        id: hash,
        delegator: delegation.delegator,
        delegatee: delegation.delegate,
        expiry: delegation.expiry,
        issued_at: delegation.issued_at,
        not_before: delegation.not_before,
        serialization,
    }))
    .exec(db)
    .await?;

    // save abilities
    abilities::Entity::insert_many(delegation.capabilities.into_iter().map(|ab| {
        abilities::ActiveModel::from(abilities::Model {
            delegation: hash,
            resource: ab.resource,
            ability: ab.action,
            caveats: Default::default(),
        })
    }))
    .exec(db)
    .await?;

    // save parent relationships
    parent_delegations::Entity::insert_many(delegation.parents.into_iter().map(|p| {
        parent_delegations::ActiveModel::from(parent_delegations::Model {
            child: hash,
            parent: p.into(),
        })
    }))
    .exec(db)
    .await?;

    Ok(hash)
}

async fn save_actor<C: ConnectionTrait>(id: String, db: &C) -> Result<(), DbErr> {
    use sea_orm::sea_query::OnConflict;
    match actor::Entity::insert(actor::ActiveModel::from(actor::Model { id }))
        .on_conflict(
            OnConflict::column(actor::Column::Id)
                .do_nothing()
                .to_owned(),
        )
        .exec(db)
        .await
    {
        Err(DbErr::RecordNotInserted) => (),
        r => {
            r?;
        }
    };
    Ok(())
}
