pub mod entities;

use sea_orm::sea_query::Index;
use sea_orm::{ConnectionTrait, DatabaseConnection, DbErr, Schema};

pub async fn init_schema(db: &DatabaseConnection) -> Result<(), DbErr> {
    let backend = db.get_database_backend();
    let schema = Schema::new(backend);

    let create_sessions = schema
        .create_table_from_entity(entities::session::Entity)
        .if_not_exists()
        .to_owned();
    let create_turns = schema
        .create_table_from_entity(entities::turn::Entity)
        .if_not_exists()
        .to_owned();
    let create_items = schema
        .create_table_from_entity(entities::item::Entity)
        .if_not_exists()
        .to_owned();
    let create_event_log = schema
        .create_table_from_entity(entities::event_log::Entity)
        .if_not_exists()
        .to_owned();
    let create_checkpoints = schema
        .create_table_from_entity(entities::checkpoint::Entity)
        .if_not_exists()
        .to_owned();

    db.execute(backend.build(&create_sessions)).await?;
    db.execute(backend.build(&create_turns)).await?;
    db.execute(backend.build(&create_items)).await?;
    db.execute(backend.build(&create_event_log)).await?;
    db.execute(backend.build(&create_checkpoints)).await?;

    let indexes = [
        Index::create()
            .name("uq_agena_turn_session_turn_index")
            .table(entities::turn::Entity)
            .col(entities::turn::Column::SessionId)
            .col(entities::turn::Column::TurnIndex)
            .unique()
            .if_not_exists()
            .to_owned(),
        Index::create()
            .name("uq_agena_item_turn_item_index")
            .table(entities::item::Entity)
            .col(entities::item::Column::TurnId)
            .col(entities::item::Column::ItemIndex)
            .unique()
            .if_not_exists()
            .to_owned(),
        Index::create()
            .name("uq_agena_event_session_seq")
            .table(entities::event_log::Entity)
            .col(entities::event_log::Column::SessionId)
            .col(entities::event_log::Column::Seq)
            .unique()
            .if_not_exists()
            .to_owned(),
        Index::create()
            .name("uq_agena_checkpoint_session_seq")
            .table(entities::checkpoint::Entity)
            .col(entities::checkpoint::Column::SessionId)
            .col(entities::checkpoint::Column::UptoSeq)
            .unique()
            .if_not_exists()
            .to_owned(),
        Index::create()
            .name("idx_agena_turn_session_started")
            .table(entities::turn::Entity)
            .col(entities::turn::Column::SessionId)
            .col(entities::turn::Column::StartedAtMs)
            .if_not_exists()
            .to_owned(),
        Index::create()
            .name("idx_agena_item_session_turn")
            .table(entities::item::Entity)
            .col(entities::item::Column::SessionId)
            .col(entities::item::Column::TurnId)
            .col(entities::item::Column::ItemIndex)
            .if_not_exists()
            .to_owned(),
        Index::create()
            .name("idx_agena_event_session_turn_seq")
            .table(entities::event_log::Entity)
            .col(entities::event_log::Column::SessionId)
            .col(entities::event_log::Column::TurnId)
            .col(entities::event_log::Column::Seq)
            .if_not_exists()
            .to_owned(),
        Index::create()
            .name("idx_agena_event_type_created")
            .table(entities::event_log::Entity)
            .col(entities::event_log::Column::EventType)
            .col(entities::event_log::Column::CreatedAtMs)
            .if_not_exists()
            .to_owned(),
        Index::create()
            .name("idx_agena_event_session_item")
            .table(entities::event_log::Entity)
            .col(entities::event_log::Column::SessionId)
            .col(entities::event_log::Column::ItemId)
            .if_not_exists()
            .to_owned(),
        Index::create()
            .name("idx_agena_checkpoint_session_created")
            .table(entities::checkpoint::Entity)
            .col(entities::checkpoint::Column::SessionId)
            .col(entities::checkpoint::Column::CreatedAtMs)
            .if_not_exists()
            .to_owned(),
    ];

    for index in indexes {
        db.execute(backend.build(&index)).await?;
    }

    Ok(())
}
