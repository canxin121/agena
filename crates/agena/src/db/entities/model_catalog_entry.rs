use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "agena_model_catalog_entries")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = true)]
    pub id: i64,
    pub kind: String,
    pub model_id: String,
    #[sea_orm(column_name = "definition_json", column_type = "JsonBinary")]
    pub definition_json: serde_json::Value,
    pub search_text: String,
    pub updated_at_ms: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
