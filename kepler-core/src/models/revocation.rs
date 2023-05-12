use super::super::{events::Revocation, models::*, util};
use crate::hash::Hash;
use kepler_lib::authorization::KeplerRevocation;
use sea_orm::{entity::prelude::*, sea_query::Condition, ConnectionTrait};
use time::OffsetDateTime;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "revocation")]
pub struct Model {
    #[sea_orm(primary_key, unique, auto_increment = false)]
    pub id: Vec<u8>,

    pub seq: u64,
    pub epoch_id: Vec<u8>,
    pub epoch_seq: u64,

    pub revoker: String,
    pub revoked: Vec<u8>,
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
        belongs_to = "epoch::Entity",
        from = "Column::EpochId",
        to = "epoch::Column::Id"
    )]
    Epoch,
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

impl Related<epoch::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Epoch.def()
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
    #[error(transparent)]
    ParameterExtraction(#[from] util::RevocationError),
    #[error("Revocation expired or not yet valid")]
    InvalidTime,
    #[error("Failed to verify signature")]
    InvalidSignature,
    #[error("Unauthorized Revoker")]
    UnauthorizedRevoker(String),
    #[error("Cannot find parent delegation")]
    MissingParents,
}

pub async fn process<C: ConnectionTrait>(
    root: &str,
    db: &C,
    revocation: Revocation,
    seq: u64,
    epoch: Hash,
    epoch_seq: u64,
) -> Result<Hash, Error> {
    let Revocation(r, serialization) = revocation;

    let t = OffsetDateTime::now_utc();

    match &r {
        KeplerRevocation::Cacao(c) => {
            c.verify()
                .await
                .map_err(|_| RevocationError::InvalidSignature)?;
            if !c.payload().valid_at(&t) {
                return Err(RevocationError::InvalidTime.into());
            };
        }
    };

    let r_info: util::RevocationInfo =
        r.try_into().map_err(RevocationError::ParameterExtraction)?;

    let hash: Hash = crate::hash::hash(&serialization);
    if !r_info.parents.is_empty() && !r_info.revoker.starts_with(root) {
        let parents = delegation::Entity::find()
            .filter(r_info.parents.iter().fold(Condition::any(), |cond, p| {
                cond.add(Column::Id.eq(p.to_bytes()))
            }))
            .all(db)
            .await?;
        if parents.len() != r_info.parents.len() {
            return Err(RevocationError::MissingParents)?;
        };

        // verify parents and get delegated capabilities
        for parent in parents {
            // get delegatee of parent
            let delegatee = parent
                .find_related(actor::Entity)
                .one(db)
                .await?
                .ok_or_else(|| RevocationError::MissingParents)?;

            if delegatee.id != r_info.revoker {
                return Err(RevocationError::UnauthorizedRevoker(delegatee.id.clone()).into());
            };
        }
    };

    ActiveModel::from(Model {
        seq,
        epoch_id: epoch.into(),
        epoch_seq,
        id: hash.into(),
        serialization,
        revoker: r_info.revoker,
        revoked: r_info.revoked.into(),
    })
    .save(db)
    .await?;

    Ok(hash)
}
