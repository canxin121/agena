use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "agena_checkpoint_blobs")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub hash: String,
    pub bytes: Vec<u8>,
    pub size_bytes: i64,
    pub created_at_ms: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
