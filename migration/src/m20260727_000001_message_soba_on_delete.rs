use sea_orm_migration::{prelude::*, schema::*};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        // SQLite ne zna kar tako dodati tujega ključa na obstoječo tabelo,
        // zato tabelo preimenujemo, ustvarimo novo (s pravilnim FK) in
        // prekopiramo stare podatke vanjo.
        db.execute_unprepared("PRAGMA foreign_keys=off;").await?;

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

        db.execute_unprepared("PRAGMA foreign_keys=on;").await?;

        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        // Vračanje nazaj na "brez FK" namenoma ni implementirano —
        // v praksi tega skoraj nikoli ne izvajamo za tovrstne popravke.
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
enum Client {
    Table,
    Id,
}

#[derive(DeriveIden)]
enum Soba {
    Table,
    Id,
}