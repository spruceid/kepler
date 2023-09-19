use super::super::{
    events::{SInvocation, SerializedEvent, VersionedOperation},
    models::*,
    relationships::*,
};
use crate::hash::Hash;
use crate::types::{Facts, OrbitIdWrap};
use kepler_lib::authorization::{Invocation, Resources};
use sea_orm::{entity::prelude::*, sea_query::OnConflict, Condition, ConnectionTrait, QueryOrder};
use time::OffsetDateTime;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "invocation")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false, unique)]
    pub id: Hash,

    pub invoker: String,
    pub issued_at: OffsetDateTime,
    pub facts: Option<Facts>,
    pub serialization: Vec<u8>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    // inverse relation, invocations belong to invokers
    #[sea_orm(
        belongs_to = "actor::Entity",
        from = "Column::Invoker",
        to = "actor::Column::Id"
    )]
    Invoker,
    #[sea_orm(has_many = "invoked_abilities::Entity")]
    InvokedAbilities,
}

impl Related<actor::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Invoker.def()
    }
}

impl Related<invoked_abilities::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::InvokedAbilities.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}

pub(crate) async fn process<C: ConnectionTrait>(
    db: &C,
    SerializedEvent(i, ser): SInvocation,
    ops: Vec<VersionedOperation>,
) -> Result<Hash, EventProcessingError> {
    let time = OffsetDateTime::now_utc();
    if !i.valid_at_time(time.unix_timestamp() as u64, None) {
        return Err(ValidationError::InvalidTime.into());
    }
    verify(&i).await?;
    validate(db, &i, Some(time)).await?;
    save(db, i, Some(time), ser, ops).await
}

async fn save<C: ConnectionTrait>(
    db: &C,
    invocation: Invocation,
    time: Option<OffsetDateTime>,
    serialization: Vec<u8>,
    parameters: Vec<VersionedOperation>,
) -> Result<Hash, EventProcessingError> {
    let hash = crate::hash::hash(&serialization);
    let issued_at = time
        .map(Ok)
        .or(invocation
            .issued_at()
            .map(|i| OffsetDateTime::from_unix_timestamp(i as i64)))
        .transpose()?
        .unwrap_or_else(OffsetDateTime::now_utc);

    match Entity::insert(ActiveModel::from(Model {
        id: hash,
        issued_at,
        serialization,
        facts: None,
        invoker: invocation.issuer().to_string(),
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

    // save invoked abilities
    if !invocation.capabilities().is_empty() {
        invoked_abilities::Entity::insert_many(
            Resources::<'_, &UriStr>::grants(&invocation)
                .map(|(resource, actions)| {
                    actions.into_iter().map(|(action, _)| {
                        invoked_abilities::ActiveModel::from(invoked_abilities::Model {
                            invocation: hash,
                            resource: resource.into(),
                            ability: action.clone().into(),
                        })
                    })
                })
                .flatten(),
        )
        .exec(db)
        .await?;
    }

    // save parent relationships
    if let Some(prf) = invocation.proof().filter(|p| !p.is_empty()) {
        parent_delegations::Entity::insert_many(prf.into_iter().map(|p| {
            parent_delegations::ActiveModel::from(parent_delegations::Model {
                child: hash,
                parent: (*p).into(),
            })
        }))
        .exec(db)
        .await?;
    }

    for param in parameters {
        match param {
            VersionedOperation::KvWrite {
                key,
                value,
                metadata,
                orbit,
                seq,
                epoch,
                epoch_seq,
            } => {
                kv_write::Entity::insert(kv_write::ActiveModel::from(kv_write::Model {
                    invocation: hash,
                    key,
                    value,
                    orbit: orbit.into(),
                    metadata,
                    seq,
                    epoch,
                    epoch_seq,
                }))
                .exec(db)
                .await?;
            }
            VersionedOperation::KvDelete {
                key,
                version,
                orbit,
            } => {
                match if let Some((s, e, es)) = version {
                    kv_write::Entity::find().filter(
                        Condition::all()
                            .add(kv_write::Column::Key.eq(key.clone()))
                            .add(kv_write::Column::Orbit.eq(OrbitIdWrap(orbit.clone())))
                            .add(kv_write::Column::Seq.eq(s))
                            .add(kv_write::Column::Epoch.eq(e))
                            .add(kv_write::Column::EpochSeq.eq(es)),
                    )
                } else {
                    kv_write::Entity::find()
                        .filter(kv_write::Column::Key.eq(key.clone()))
                        .filter(kv_write::Column::Orbit.eq(OrbitIdWrap(orbit.clone())))
                        .order_by_desc(kv_write::Column::Seq)
                        .order_by_desc(kv_write::Column::Epoch)
                        .order_by_desc(kv_write::Column::EpochSeq)
                }
                .one(db)
                .await?
                {
                    Some(kv) => Ok(kv_delete::Entity::insert(kv_delete::ActiveModel::from(
                        kv_delete::Model {
                            key,
                            invocation_id: hash,
                            orbit: orbit.into(),
                            deleted_invocation_id: kv.invocation,
                        },
                    ))
                    .exec(db)),
                    None => Err(EventProcessingError::MissingServiceEvent(
                        orbit.to_resource(Some("kv".to_string()), Some(key), None),
                        "kv/del".to_string(),
                        version,
                    )),
                }?
                .await?;
            }
        }
    }

    Ok(hash)
}
