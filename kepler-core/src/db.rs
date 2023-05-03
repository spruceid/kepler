use sea_orm::{
    error::DbErr, ConnectOptions, Database, DatabaseConnection, Schema, ConnectionTrait, entity::prelude::*, query::QuerySelect
};
use crate::models::*;
use crate::events::Epoch;

pub struct OrbitDatabase {
    conn: DatabaseConnection,
}

pub struct Commit{
    pub hash: [u8; 32],
    pub seq: u64,
    pub commited_events: Vec<[u8; 32]>
}

impl OrbitDatabase {
    pub async fn new<C: Into<ConnectOptions>>(options: C) -> Result<Self, DbErr> {
        Ok(Self { conn: Database::connect(options).await? })
    }

    pub async fn setup_tables(&self) -> Result<(), DbErr> {
        let db_backend = self.conn.get_database_backend();
        let schema = Schema::new(db_backend);
        self.conn.execute(
            db_backend
                .build(&schema.create_table_from_entity(epoch::Entity)))
          .await?;
        self.conn.execute(
            db_backend
                .build(&schema.create_table_from_entity(delegation::Entity)))
          .await?;
        self.conn.execute(
            db_backend
                .build(&schema.create_table_from_entity(invocation::Entity)))
          .await?;
        self.conn.execute(
            db_backend
                .build(&schema.create_table_from_entity(revocation::Entity)))
          .await?;
        self.conn.execute(
            db_backend
                .build(&schema.create_table_from_entity(actor::Entity)))
          .await?;
        Ok(())
    }

    pub async fn get_max_seq(&self) -> Result<u64, DbErr> {
        Ok(epoch::Entity::find().select_only().column(epoch::Column::Seq.max()).into_tuple().one(&self.conn).await?.unwrap_or(0))
    }

    pub async fn get_most_recent(&self) -> Result<Vec<[u8; 32]>, DbErr> {
        todo!()
    }

    pub async fn new_epoch(&self) -> Result<Epoch, DbErr> {
        let seq = self.get_max_seq().await?;
        // TODO get max sequence number from db
        // TODO get hashes of epochs without children from db
        Ok(Epoch::new(seq + 1, todo!("get parents from db")))
    }

    pub async fn process_epoch(&self, epoch: Epoch) -> Result<Commit, DbErr> {
        let (seq, parents, events) = epoch.into_inner();
        // TODO verify all events/signatures
        // write events to db
        // write epoch to db
        // update all the stuff
        todo!()
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use async_std::test;

    async fn get_db() -> Result<OrbitDatabase, DbErr> {
        OrbitDatabase::new("sqlite::memory:").await
    }

    #[test]
    async fn basic() {
        let db = get_db().await.unwrap();
    }
}
