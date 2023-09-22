use crate::hash::Hash;
use crate::types::Facts;
use crate::{
    events::{SDelegation, SerializedEvent},
    models::*,
    relationships::*,
};
use kepler_lib::authorization::{delegation_from_bytes, Delegation, EncodingError, Resources};
use sea_orm::{entity::prelude::*, sea_query::OnConflict, ConnectionTrait};
use time::{Duration, OffsetDateTime};

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
    pub facts: Option<Facts>,
    pub serialization: Vec<u8>,
}

impl Model {
    pub(crate) fn reser_cacao(&self) -> Result<SDelegation, EncodingError> {
        Ok(SerializedEvent(
            delegation_from_bytes(&self.serialization)?,
            self.serialization.clone(),
        ))
    }

    pub(crate) fn valid_at(&self, time: OffsetDateTime, skew: Option<Duration>) -> bool {
        let skew = skew.unwrap_or_else(|| Duration::seconds(0));
        self.expiry.map_or(true, |exp| time < exp + skew)
            && self.not_before.map_or(true, |nbf| nbf <= time + skew)
    }

    pub(crate) fn validate_bounds(
        &self,
        start: Option<OffsetDateTime>,
        end: Option<OffsetDateTime>,
    ) -> bool {
        let a = match (self.not_before, start) {
            (Some(nbf), Some(start)) => start >= nbf,
            (None, Some(_)) => false,
            _ => true,
        };
        let b = match (self.expiry, end) {
            (Some(exp), Some(end)) => exp >= end,
            (None, Some(_)) => false,
            _ => true,
        };
        a && b
    }
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
    #[sea_orm(has_many = "revocation::Entity")]
    Revocation,
    #[sea_orm(has_many = "abilities::Entity")]
    Abilities,
    #[sea_orm(has_many = "parent_delegations::Entity")]
    Parents,
}

impl Related<actor::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Delegator.def()
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

pub(crate) async fn process<C: ConnectionTrait>(
    db: &C,
    SerializedEvent(d, ser): SDelegation,
) -> Result<Hash, EventProcessingError> {
    let time = OffsetDateTime::now_utc();
    if !d.valid_at_time(time.unix_timestamp() as u64, None) {
        return Err(ValidationError::InvalidTime.into());
    }
    verify(&d).await?;
    validate(db, &d, None).await?;

    save(db, d, ser).await
}

async fn save<C: ConnectionTrait>(
    db: &C,
    delegation: Delegation,
    serialization: Vec<u8>,
) -> Result<Hash, EventProcessingError> {
    save_actors(
        &[
            &delegation.issuer().to_string(),
            &delegation.audience().to_string(),
        ],
        db,
    )
    .await?;

    let hash: Hash = crate::hash::hash(&serialization);

    // save delegation
    match Entity::insert(ActiveModel::from(Model {
        id: hash,
        delegator: delegation.issuer().to_string(),
        delegatee: delegation.audience().to_string(),
        expiry: delegation
            .expiration()
            .map(|i| OffsetDateTime::from_unix_timestamp(i as i64))
            .transpose()
            .map_err(ValidationError::from)?,
        issued_at: delegation
            .issued_at()
            .map(|i| OffsetDateTime::from_unix_timestamp(i as i64))
            .transpose()
            .map_err(ValidationError::from)?,
        not_before: delegation
            .not_before()
            .map(|i| OffsetDateTime::from_unix_timestamp(i as i64))
            .transpose()
            .map_err(ValidationError::from)?,
        facts: delegation
            .facts()
            // TODO not ideal
            .map(|f| serde_json::from_value(serde_json::to_value(f)?))
            .transpose()?,
        serialization,
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

    // save abilities
    if !delegation.capabilities().is_empty() {
        let abilities = Resources::<'_, AnyResource>::grants(&delegation)
            .flat_map(|(resource, abilities)| {
                abilities.iter().map(move |(ability, c)| abilities::Model {
                    delegation: hash,
                    resource: resource.clone().into(),
                    ability: ability.clone().into(),
                    caveats: c.clone().into(),
                })
            })
            .map(abilities::ActiveModel::from)
            .collect::<Vec<_>>();
        abilities::Entity::insert_many(abilities).exec(db).await?;
    }

    // save parent relationships
    if let Some(prf) = delegation.proof().filter(|p| !p.is_empty()) {
        parent_delegations::Entity::insert_many(prf.iter().map(|p| {
            parent_delegations::ActiveModel::from(parent_delegations::Model {
                child: hash,
                parent: (*p).into(),
            })
        }))
        .exec(db)
        .await?;
    }

    Ok(hash)
}

async fn save_actors<C: ConnectionTrait>(actors: &[&str], db: &C) -> Result<(), DbErr> {
    match actor::Entity::insert_many(
        actors
            .iter()
            .map(|a| actor::ActiveModel::from(actor::Model { id: a.to_string() })),
    )
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
