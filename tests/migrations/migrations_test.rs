use crate::integration::test_app;

use chat_room_prog2::{
    controller::{
        forms::{admin_reset_legacy_password, verify_password},
        rooms::{ensure_default_room, prepare_database_schema},
    },
    entities::{
        message,
        prelude::{Client, Soba},
        soba,
    },
};
use migration::{Migrator, MigratorTrait};
use sea_orm::{
    ColumnTrait, ConnectOptions, ConnectionTrait, Database, EntityTrait, PaginatorTrait,
    QueryFilter,
};

#[tokio::test]
async fn migrations_and_default_room_are_idempotent() {
    let (_app, db) = test_app().await;

    prepare_database_schema(&db).await.unwrap();
    ensure_default_room(&db).await.unwrap();
    ensure_default_room(&db).await.unwrap();

    assert_eq!(
        Soba::find()
            .filter(soba::Column::Name.eq("general"))
            .count(&db)
            .await
            .unwrap(),
        1
    );
}

#[tokio::test]
async fn migrations_preserve_data_from_a_populated_legacy_database() {
    let mut options = ConnectOptions::new("sqlite::memory:");
    options.max_connections(1).sqlx_logging(false);
    let db = Database::connect(options).await.unwrap();

    // Prve tri migracije predstavljajo staro različico baze: uporabnik še nima
    // gesla, sporočilo pa še ni povezano s sobo.
    Migrator::up(&db, Some(3)).await.unwrap();
    db.execute_unprepared("INSERT INTO client (id, username) VALUES (1, 'Stari_Uporabnik')")
        .await
        .unwrap();
    db.execute_unprepared("INSERT INTO soba (id, name) VALUES (123456, 'stara_soba')")
        .await
        .unwrap();
    db.execute_unprepared(
        "INSERT INTO message (id, sender_id, content, timestamp) \
         VALUES (1, 1, 'staro sporocilo', 1700000000)",
    )
    .await
    .unwrap();

    prepare_database_schema(&db).await.unwrap();

    let legacy_user = Client::find_by_id(1).one(&db).await.unwrap().unwrap();
    assert_eq!(legacy_user.username, "Stari_Uporabnik");
    assert_eq!(legacy_user.geslo, "");
    assert!(!verify_password("karkoli", &legacy_user.geslo).unwrap());

    assert!(
        admin_reset_legacy_password(&db, "Stari_Uporabnik", None, "12345")
            .await
            .is_err()
    );
    let reset_user = admin_reset_legacy_password(&db, "Stari_Uporabnik", None, "nova_skrivnost")
        .await
        .unwrap();
    assert_eq!(reset_user.username, "stari_uporabnik");
    assert!(verify_password("nova_skrivnost", &reset_user.geslo).unwrap());
    assert!(
        admin_reset_legacy_password(&db, "stari_uporabnik", None, "druga_skrivnost")
            .await
            .is_err()
    );

    let general = Soba::find()
        .filter(soba::Column::Name.eq("general"))
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert!(Soba::find_by_id(123456).one(&db).await.unwrap().is_some());

    let legacy_message = message::Entity::find_by_id(1)
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(legacy_message.content, "staro sporocilo");
    assert_eq!(legacy_message.sender_id, Some(1));
    assert_eq!(legacy_message.soba_id, general.id);
}
