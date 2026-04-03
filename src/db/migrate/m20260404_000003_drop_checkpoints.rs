use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const SESSION_CHECKPOINTS: &str = "agena_session_checkpoints";
const SESSION_RESTORE_POINTS: &str = "agena_session_restore_points";
const CHECKPOINT_BLOBS: &str = "agena_checkpoint_blobs";

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for table in [CHECKPOINT_BLOBS, SESSION_RESTORE_POINTS, SESSION_CHECKPOINTS] {
            manager
                .drop_table(Table::drop().if_exists().table(Alias::new(table)).to_owned())
                .await?;
        }

        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Ok(())
    }
}
