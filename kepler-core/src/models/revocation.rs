use super::super::{events::SRevocation, models::*};
use crate::hash::{hash, Hash};
use kepler_lib::resolver::DID_METHODS;
use sea_orm::{entity::prelude::*, sea_query::OnConflict, ConnectionTrait};

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "revocation")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false, unique)]
    pub id: Hash,

    pub revoker: String,
    pub revoked: Hash,
    pub serialization: Vec<u8>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "actor::Entity",
        from = "Column::Revoker",
        to = "actor::Column::Id"
    )]
    Revoker,
    #[sea_orm(
        belongs_to = "delegation::Entity",
        from = "Column::Revoked",
        to = "delegation::Column::Id"
    )]
    Delegation,
}

impl Related<actor::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Revoker.def()
    }
}

impl Related<delegation::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Delegation.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Db(#[from] DbErr),
    #[error(transparent)]
    InvalidRevocation(#[from] RevocationError),
}

#[derive(Debug, thiserror::Error)]
pub enum RevocationError {
    #[error("Revocation expired or not yet valid")]
    InvalidTime,
    #[error("Failed to verify signature")]
    InvalidSignature,
    #[error("Unauthorized Revoker")]
    UnauthorizedRevoker(String),
    #[error("Cannot find parent delegation")]
    MissingParents,
}

pub(crate) async fn process<C: ConnectionTrait>(
    db: &C,
    revocation: SRevocation,
) -> Result<Hash, Error> {
    let (r, serialization) = (revocation.0, revocation.1);

    r.verify_signature(&*DID_METHODS, None)
        .await
        .map_err(|_| RevocationError::InvalidSignature)?;

    let hash: Hash = hash(&serialization);
    // TODO get the whole delegation chain
    let delegation = delegation::Entity::find_by_id(Hash::from(r.revoke))
        .one(db)
        .await?
        .ok_or(RevocationError::MissingParents)?;

    // check the revoker is also the delegator
    if delegation.delegator != r.issuer {
        return Err(RevocationError::UnauthorizedRevoker(r.issuer).into());
    };

    match Entity::insert(ActiveModel::from(Model {
        id: hash,
        serialization,
        revoker: r.issuer,
        revoked: r.revoke.into(),
    }))
    .on_conflict(OnConflict::column(Column::Id).do_nothing().to_owned())
    .exec(db)
    .await
    {
        Err(DbErr::RecordNotInserted) => return Ok(hash),
        r => {
            r?;
        }
    };

    Ok(hash)
}
