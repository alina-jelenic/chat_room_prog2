use sea_orm_migration::{prelude::*, schema::*};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(MessageReaction::Table)
                    .if_not_exists()
                    .col(pk_auto(MessageReaction::Id))
                    .col(integer(MessageReaction::MessageId))
                    .col(integer(MessageReaction::ClientId))
                    .col(string(MessageReaction::Emoji))
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk-reaction-message")
                            .from(MessageReaction::Table, MessageReaction::MessageId)
                            .to(Message::Table, Message::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk-reaction-client")
                            .from(MessageReaction::Table, MessageReaction::ClientId)
                            .to(Client::Table, Client::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        // Ker je to unikatno potem ko še enkrat klikne na emoji se samo izbriše
        manager
            .create_index(
                Index::create()
                    .name("idx-reaction-unique")
                    .table(MessageReaction::Table)
                    .col(MessageReaction::MessageId)
                    .col(MessageReaction::ClientId)
                    .col(MessageReaction::Emoji)
                    .unique()
                    .if_not_exists()
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(MessageReaction::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum MessageReaction {
    Table,
    Id,
    MessageId,
    ClientId,
    Emoji,
}

#[derive(DeriveIden)]
enum Message {
    Table,
    Id,
}

#[derive(DeriveIden)]
enum Client {
    Table,
    Id,
}
