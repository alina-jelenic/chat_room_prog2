use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Stolpec je najprej lahko NULL, da migracija uspe tudi na stari bazi,
        // ki že vsebuje sporočila. Poznejša migracija manjkajoče vrednosti
        // prestavi v #general in nato ustvari končno obvezno FK-relacijo.
        manager
            .get_connection()
            .execute_unprepared("ALTER TABLE message ADD COLUMN soba_id INTEGER")
            .await?;
        Ok(())
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
