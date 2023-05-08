use super::super::{events::Delegation, util::DelegationInfo};
use crate::db::TxError;
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
        belongs_to = "super::actor::Entity",
        from = "Column::Delegator",
        to = "super::actor::Column::Id"
    )]
    Delegator,
    #[sea_orm(has_one = "super::actor::Entity")]
    Delegatee,
    // inverse relation, delegations belong to epochs
    #[sea_orm(
        belongs_to = "super::epoch::Entity",
        from = "Column::Id",
        to = "super::epoch::Column::Id"
    )]
    Epoch,
    #[sea_orm(has_many = "super::invocation::Entity")]
    Invocation,
    #[sea_orm(has_many = "super::revocation::Entity")]
    Revocation,
}

impl Related<super::actor::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Delegator.def()
    }
}

impl Related<super::epoch::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Epoch.def()
    }
}

impl Related<super::invocation::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Invocation.def()
    }
}

impl Related<super::revocation::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Revocation.def()
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

pub async fn process<C: ConnectionTrait>(
    root: &str,
    db: &C,
    delegation: Delegation,
) -> Result<[u8; 32], TxError> {
    let Delegation(d, ser) = delegation;
    // verify signatures
    match d {
        KeplerDelegation::Ucan(ucan) => {
            ucan.verify_signature(DID_METHODS.to_resolver()).await?;
            ucan.payload.validate_time(None)?;
        }
        KeplerDelegation::Cacao(cacao) => {
            cacao.verify().await?;
            if !cacao.payload().valid_now() {
                return Err(TxError::InvalidDelegation);
            }
        }
    };
    let d_info = DelegationInfo::try_from(d)?;
    if !d_info.parents.is_empty() || !d_info.delegator.starts_with(root) {
        let parents = Entity::find()
            .filter(d_info.parents.iter().fold(Condition::any(), |cond, p| {
                cond.add(Column::Id.eq(p.to_bytes()))
            }))
            .all(db)
            .await?;
        for parent in parents {
            let delegatee = parent
                .find_with_related(Relation::Delegatee)
                .all(db)
                .await?;
            if delegatee != d_info.delegator {
                return Err(TxError::InvalidDelegation);
            };
            if parent.expiry < d_info.expiry {
                return Err(TxError::InvalidDelegation);
            };
            // parent nbf must come before child nbf, child nbf must exist if parent nbf does
            if parent
                .not_before
                .map(|pnbf| d_info.not_before.map(|nbf| pnbf > nbf).unwrap_or(true))
                .unwrap_or(false)
            {
                return Err(TxError::InvalidDelegation);
            };
            // TODO check attenuations
        }
    }

    let hash = todo!();
    Ok(hash)
}
