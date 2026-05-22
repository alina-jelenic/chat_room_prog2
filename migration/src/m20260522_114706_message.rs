use sea_orm_migration::{prelude::*, schema::*};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Message::Table)
                    .if_not_exists()
                    .col(pk_auto(Message::Id))
                    .col(big_unsigned_null(Message::SenderId))
                    .col(text(Message::Content))
                    .col(big_unsigned(Message::Timestamp))
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk-message-sender")
                            .from(Message::Table, Message::SenderId)
                            .to(Client::Table, Client::Id)
                            .on_delete(ForeignKeyAction::SetNull),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(Message::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum Message {
    Table,
    Id,
    SenderId,
    Content,
    Timestamp,
}

#[derive(DeriveIden)]
enum Client {
    Table,
    Id,
}