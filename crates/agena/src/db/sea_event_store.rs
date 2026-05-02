use std::marker::PhantomData;
use std::sync::Arc;

use async_trait::async_trait;
use sea_orm::sea_query::{Expr, Order};
use sea_orm::{
    ActiveValue, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder,
    QuerySelect, TransactionTrait,
};

use crate::event::envelope::DomainEvent;
use crate::event::{EventFilter, EventStore, EventStoreError, KindMatcher, Scope, StoreRange};

use crate::db::event_entity as entity;

/// sea_orm-backed [`EventStore`].
///
/// Generic over the concrete `EventKind` enum `K`. Persists the full
/// `DomainEvent<K>` envelope as JSONB; indexed columns mirror the meta fields.
pub struct SeaEventStore<K> {
    db: Arc<DatabaseConnection>,
    _phantom: PhantomData<fn() -> K>,
}

impl<K> SeaEventStore<K> {
    pub fn new(db: Arc<DatabaseConnection>) -> Self {
        Self {
            db,
            _phantom: PhantomData,
        }
    }

    pub fn db(&self) -> &Arc<DatabaseConnection> {
        &self.db
    }
}

fn map_db_err(err: sea_orm::DbErr) -> EventStoreError {
    EventStoreError::Backend(err.to_string())
}

fn into_active_model<K>(event: &DomainEvent<K>) -> Result<entity::ActiveModel, EventStoreError>
where
    K: KindMatcher + serde::Serialize,
{
    let payload = serde_json::to_value(event)?;
    let tag = event.kind.tag();
    Ok(entity::ActiveModel {
        id: ActiveValue::NotSet,
        event_uuid: ActiveValue::Set(event.meta.id.to_string()),
        seq_global: ActiveValue::Set(event.meta.seq_global),
        seq_session: ActiveValue::Set(event.meta.seq_session),
        session_id: ActiveValue::Set(event.meta.session_id),
        workspace_id: ActiveValue::Set(event.meta.workspace_id),
        kind_tag: ActiveValue::Set(tag.to_string()),
        envelope_schema: ActiveValue::Set(event.meta.envelope_schema as i32),
        payload: ActiveValue::Set(payload),
        causation_uuid: ActiveValue::Set(event.meta.causation_id.map(|u| u.to_string())),
        correlation_uuid: ActiveValue::Set(event.meta.correlation_id.map(|u| u.to_string())),
        created_at_ms: ActiveValue::Set(event.meta.created_at.timestamp_millis()),
    })
}

fn from_model<K>(model: entity::Model) -> Result<DomainEvent<K>, EventStoreError>
where
    K: serde::de::DeserializeOwned,
{
    serde_json::from_value::<DomainEvent<K>>(model.payload).map_err(EventStoreError::Serde)
}

#[async_trait]
impl<K> EventStore<K> for SeaEventStore<K>
where
    K: KindMatcher + Clone + Send + Sync + 'static + serde::Serialize + serde::de::DeserializeOwned,
{
    async fn append_batch(&self, events: &[DomainEvent<K>]) -> Result<(), EventStoreError> {
        if events.is_empty() {
            return Ok(());
        }
        let txn = self.db.begin().await.map_err(map_db_err)?;
        let mut models = Vec::with_capacity(events.len());
        for ev in events {
            models.push(into_active_model(ev)?);
        }
        entity::Entity::insert_many(models)
            .exec(&txn)
            .await
            .map_err(|err| match err {
                sea_orm::DbErr::Exec(_) | sea_orm::DbErr::Query(_) => {
                    EventStoreError::Backend(err.to_string())
                }
                other => map_db_err(other),
            })?;
        txn.commit().await.map_err(map_db_err)?;
        Ok(())
    }

    async fn range(
        &self,
        filter: &EventFilter,
        range: StoreRange,
    ) -> Result<Vec<DomainEvent<K>>, EventStoreError> {
        let mut query = entity::Entity::find()
            .filter(entity::Column::SeqGlobal.gt(range.after_seq_global))
            .order_by(entity::Column::SeqGlobal, Order::Asc)
            .limit(range.limit as u64);

        match &filter.scope {
            Scope::Global => {}
            Scope::Workspace { workspace_id } => {
                query = query.filter(entity::Column::WorkspaceId.eq(*workspace_id));
            }
            Scope::Session { session_id } => {
                query = query.filter(entity::Column::SessionId.eq(*session_id));
            }
        }

        if let Some(kinds) = &filter.kinds {
            let tags: Vec<String> = kinds.iter().map(|t| t.to_string()).collect();
            query = query.filter(entity::Column::KindTag.is_in(tags));
        }

        if let Some(since) = filter.since_seq_global {
            query = query.filter(entity::Column::SeqGlobal.gt(since));
        }

        let models = query.all(self.db.as_ref()).await.map_err(map_db_err)?;
        let mut out = Vec::with_capacity(models.len());
        for model in models {
            out.push(from_model(model)?);
        }
        Ok(out)
    }

    async fn high_watermark(&self) -> Result<Option<i64>, EventStoreError> {
        let row: Option<(Option<i64>,)> = entity::Entity::find()
            .select_only()
            .column_as(Expr::col(entity::Column::SeqGlobal).max(), "max_seq")
            .into_tuple()
            .one(self.db.as_ref())
            .await
            .map_err(map_db_err)?;
        Ok(row.and_then(|(v,)| v))
    }

    async fn session_high_watermark(
        &self,
        session_id: i64,
    ) -> Result<Option<i64>, EventStoreError> {
        let row: Option<(Option<i64>,)> = entity::Entity::find()
            .filter(entity::Column::SessionId.eq(session_id))
            .select_only()
            .column_as(Expr::col(entity::Column::SeqSession).max(), "max_seq")
            .into_tuple()
            .one(self.db.as_ref())
            .await
            .map_err(map_db_err)?;
        Ok(row.and_then(|(v,)| v))
    }
}
