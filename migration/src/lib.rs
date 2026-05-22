pub use sea_orm_migration::prelude::*;

mod m20260522_114626_soba;
mod m20260522_114706_message;
mod m20260522_114715_client;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20260522_114626_soba::Migration),
            Box::new(m20260522_114706_message::Migration),
            Box::new(m20260522_114715_client::Migration),
        ]
    }
}
