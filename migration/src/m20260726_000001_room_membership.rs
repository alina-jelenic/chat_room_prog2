use sea_orm_migration::{prelude::*, schema::*};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Soba::Table)
                    .add_column(integer_null(Soba::OwnerId))
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(RoomMember::Table)
                    .if_not_exists()
                    .col(pk_auto(RoomMember::Id))
                    .col(integer(RoomMember::SobaId))
                    .col(integer(RoomMember::ClientId))
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk-room-member-soba")
                            .from(RoomMember::Table, RoomMember::SobaId)
                            .to(Soba::Table, Soba::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk-room-member-client")
                            .from(RoomMember::Table, RoomMember::ClientId)
                            .to(Client::Table, Client::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx-room-member-unique")
                    .table(RoomMember::Table)
                    .col(RoomMember::SobaId)
                    .col(RoomMember::ClientId)
                    .unique()
                    .if_not_exists()
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(RoomMember::Table).to_owned())
            .await?;

        manager
            .alter_table(
                Table::alter()
                    .table(Soba::Table)
                    .drop_column(Soba::OwnerId)
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum Soba {
    Table,
    Id,
    OwnerId,
}

#[derive(DeriveIden)]
enum Client {
    Table,
    Id,
}

#[derive(DeriveIden)]
enum RoomMember {
    Table,
    Id,
    SobaId,
    ClientId,
}
