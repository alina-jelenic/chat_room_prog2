use sea_orm_migration::{prelude::*, schema::*};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        // Stara baza morda še nima sobe #general. Ustvarimo jo z naslednjim
        // prostim ID-jem, nato pa vanjo prestavimo sporočila brez veljavne sobe.
        db.execute_unprepared(
            "INSERT INTO soba (id, name, owner_id) \
             SELECT COALESCE((SELECT MAX(id) + 1 FROM soba), 100000), 'general', NULL \
             WHERE NOT EXISTS (SELECT 1 FROM soba WHERE name = 'general')",
        )
        .await?;
        db.execute_unprepared(
            "UPDATE message \
             SET soba_id = (SELECT id FROM soba WHERE name = 'general') \
             WHERE soba_id IS NULL \
                OR NOT EXISTS (SELECT 1 FROM soba WHERE soba.id = message.soba_id)",
        )
        .await?;
        db.execute_unprepared(
            "UPDATE message \
             SET sender_id = NULL \
             WHERE sender_id IS NOT NULL \
               AND NOT EXISTS (SELECT 1 FROM client WHERE client.id = message.sender_id)",
        )
        .await?;

        // SQLite ne zna naknadno dodati tujega ključa, zato tabelo preimenujemo,
        // ustvarimo pravilno končno tabelo in vanjo prekopiramo očiščene podatke.

        manager
            .rename_table(
                Table::rename()
                    .table(Message::Table, MessageOld::Table)
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(Message::Table)
                    .if_not_exists()
                    .col(pk_auto(Message::Id))
                    .col(big_unsigned_null(Message::SenderId))
                    .col(text(Message::Content))
                    .col(big_unsigned(Message::Timestamp))
                    .col(integer(Message::SobaId))
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk-message-sender")
                            .from(Message::Table, Message::SenderId)
                            .to(Client::Table, Client::Id)
                            .on_delete(ForeignKeyAction::SetNull),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk-message-soba")
                            .from(Message::Table, Message::SobaId)
                            .to(Soba::Table, Soba::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;
        db.execute_unprepared(
            "INSERT INTO message (id, sender_id, content, timestamp, soba_id) \
             SELECT id, sender_id, content, timestamp, soba_id FROM message_old",
        )
        .await?;

        manager
            .drop_table(Table::drop().table(MessageOld::Table).to_owned())
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        manager
            .rename_table(
                Table::rename()
                    .table(Message::Table, MessageWithRoomFk::Table)
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(Message::Table)
                    .col(pk_auto(Message::Id))
                    .col(big_unsigned_null(Message::SenderId))
                    .col(text(Message::Content))
                    .col(big_unsigned(Message::Timestamp))
                    .col(integer_null(Message::SobaId))
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk-message-sender")
                            .from(Message::Table, Message::SenderId)
                            .to(Client::Table, Client::Id)
                            .on_delete(ForeignKeyAction::SetNull),
                    )
                    .to_owned(),
            )
            .await?;

        db.execute_unprepared(
            "INSERT INTO message (id, sender_id, content, timestamp, soba_id) \
             SELECT id, sender_id, content, timestamp, soba_id \
             FROM message_with_room_fk",
        )
        .await?;

        manager
            .drop_table(Table::drop().table(MessageWithRoomFk::Table).to_owned())
            .await?;

        Ok(())
    }
}

#[derive(DeriveIden)]
enum Message {
    Table,
    Id,
    SenderId,
    Content,
    Timestamp,
    SobaId,
}

#[derive(DeriveIden)]
enum MessageOld {
    Table,
}

#[derive(DeriveIden)]
enum MessageWithRoomFk {
    Table,
}

#[derive(DeriveIden)]
enum Client {
    Table,
    Id,
}

#[derive(DeriveIden)]
enum Soba {
    Table,
    Id,
}
