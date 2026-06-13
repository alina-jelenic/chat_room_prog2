use sea_orm_migration::{prelude::*, schema::*};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
        .alter_table(
            Table::alter()
                .table(Message::Table)
                .add_column(integer(Message::SobaId))
                .to_owned(),
        )
        .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
        .alter_table(
            Table::alter()
            .table(Message::Table)
            .drop_column(Message::SobaId)
            .to_owned(),
        )
        .await
    }
}   

#[derive(DeriveIden)]
enum Message {
    Table,
    SobaId,
}