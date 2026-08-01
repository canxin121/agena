use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use sea_orm::{DatabaseConnection, DatabaseTransaction, DbErr};
use tokio::sync::{Mutex as AsyncMutex, OnceCell};

use crate::{
    AppError,
    db::crud::session,
    event::{EventKind, EventPublisher, MessagePartCheckpointedEvent},
    message::Message,
};
use agena_domain::{PermissionMode, PermissionRuleEvent, PermissionScope};
use agena_storage::{
    GlobalIdAllocator, PermissionRuleRepository, PermissionRuleTransactionWriter,
    PersistedPermissionRule, SessionSummaryRepository, WorkspaceRepository,
};

use super::{
    Session,
    cache::{SessionCache, SessionCachePolicy},
    history::SessionHistoryStore,
};
use agena_domain::SessionCacheStats;

pub(crate) struct SessionCommit {
    pub(crate) session: Session,
    pub(crate) checkpoints: Vec<MessageCheckpoint>,
    pub(crate) client_events: Vec<EventKind>,
    pub(crate) persisted_rules: Vec<PersistedPermissionRule>,
}

/// Explicit delta for durable model-message projection.
///
/// A commit names the exact parts whose value or status changed. This prevents
/// an update to one streamed Operation from checkpointing every older sibling
/// in the same assistant message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MessageCheckpoint {
    pub(crate) message_id: i64,
    pub(crate) part_ids: Vec<i64>,
}

impl MessageCheckpoint {
    pub(crate) fn all(message: &Message) -> Self {
        Self::parts(message.id, message.parts.iter().map(|part| part.id))
    }

    pub(crate) fn part(message_id: i64, part_id: i64) -> Self {
        Self {
            message_id,
            part_ids: vec![part_id],
        }
    }

    pub(crate) fn parts(message_id: i64, part_ids: impl IntoIterator<Item = i64>) -> Self {
        let mut part_ids = part_ids.into_iter().collect::<Vec<_>>();
        part_ids.sort_unstable();
        part_ids.dedup();
        Self {
            message_id,
            part_ids,
        }
    }
}

#[derive(Debug, Clone)]
struct PersistedRuleEventMeta {
    pub(crate) rule: PersistedPermissionRule,
    pub(crate) rule_id: i64,
    pub(crate) created: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct ProcessorPartIdAllocator {
    pub(crate) ids: Arc<AsyncMutex<GlobalIdAllocator>>,
}

impl ProcessorPartIdAllocator {
    pub(crate) fn new(ids: Arc<AsyncMutex<GlobalIdAllocator>>) -> Self {
        Self { ids }
    }

    pub(crate) async fn reserve(&self) -> Result<i64, AppError> {
        let mut allocator = self.ids.lock().await;
        if !allocator.initialized {
            return Err(AppError::Internal(
                "processor part allocator used before initialization".to_string(),
            ));
        }

        let part_id = allocator.next_part_id;
        allocator.next_part_id += 1;
        Ok(part_id)
    }
}

pub(crate) struct SessionStore {
    pub(crate) db: DatabaseConnection,
    pub(crate) workspace_path: String,
    pub(crate) workspace_id: OnceCell<i64>,
    pub(crate) cache: Arc<Mutex<SessionCache>>,
    pub(crate) ids: Arc<AsyncMutex<GlobalIdAllocator>>,
    pub(crate) history: SessionHistoryStore,
    pub(crate) publisher: Arc<EventPublisher>,
    pub(crate) workspace_repository: Arc<dyn WorkspaceRepository>,
    pub(crate) permission_rule_repository: Arc<dyn PermissionRuleRepository>,
    pub(crate) permission_rule_transaction_writer:
        Arc<dyn PermissionRuleTransactionWriter<DatabaseTransaction>>,
    pub(crate) session_stats_repository: Arc<dyn agena_storage::SessionStatsRepository>,
    pub(crate) usage_repository: Arc<dyn agena_storage::UsageRepository>,
    pub(crate) session_mutation_repository: Arc<dyn agena_storage::SessionMutationRepository>,
    pub(crate) projection_lookup_repository: Arc<dyn agena_storage::ProjectionLookupRepository>,
    pub(crate) session_summary_repository: Arc<dyn SessionSummaryRepository>,
}

mod core;
mod event_rewrite;
mod helpers;
mod history;
mod ids;
mod types;
mod workspace;

pub(crate) use self::event_rewrite::*;
pub(crate) use self::helpers::*;
pub(crate) use self::types::*;
