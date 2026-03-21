pub mod entities;
mod message_repository;

use sea_orm::{ConnectionTrait, DatabaseConnection, Schema};

pub use message_repository::MessageRepository;

pub async fn init_schema(db: &DatabaseConnection) -> Result<(), sea_orm::DbErr> {
    let backend = db.get_database_backend();
    let schema = Schema::new(backend);
    let create_messages = schema
        .create_table_from_entity(entities::message::Entity)
        .if_not_exists()
        .to_owned();
    let create_parts = schema
        .create_table_from_entity(entities::part::Entity)
        .if_not_exists()
        .to_owned();
    db.execute(backend.build(&create_messages)).await?;
    db.execute(backend.build(&create_parts)).await?;
    Ok(())
}
