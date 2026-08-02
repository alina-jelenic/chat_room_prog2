use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Stare različice so lahko že vsebovale uporabnike. SQLite ne dovoli
        // dodajanja obveznega stolpca v neprazno tabelo brez privzete vrednosti.
        // Prazen hash označi račun, ki potrebuje ločeno ponastavitev gesla;
        // prijava ga obravnava kot neveljavno geslo, ne kot napako strežnika.
        manager
            .get_connection()
            .execute_unprepared("ALTER TABLE client ADD COLUMN geslo TEXT NOT NULL DEFAULT ''")
            .await?;
        Ok(())
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
