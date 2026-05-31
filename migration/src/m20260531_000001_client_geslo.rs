use sea_orm_migration::{prelude::*, schema::*};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Client::Table)
                    .add_column(string(Client::Geslo))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Client::Table)
                    .drop_column(Client::Geslo)
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum Client {
    Table,
    Geslo,
}