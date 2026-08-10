//! SeaORM model članstva uporabnika v sobi.

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[sea_orm(table_name = "room_member")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub soba_id: i32,
    pub client_id: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::soba::Entity",
        from = "Column::SobaId",
        to = "super::soba::Column::Id",
        on_update = "NoAction",
        on_delete = "Cascade"
    )]
    Soba,
    #[sea_orm(
        belongs_to = "super::client::Entity",
        from = "Column::ClientId",
        to = "super::client::Column::Id",
        on_update = "NoAction",
        on_delete = "Cascade"
    )]
    Client,
}

impl Related<super::soba::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Soba.def()
    }
}

impl Related<super::client::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Client.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
