use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use sea_orm::{DatabaseConnection, DbErr};
use tokio::sync::{Mutex as AsyncMutex, OnceCell};

use crate::{
    AppError,
    db::{
        crud::{permission_rule, session, workspace as workspace_crud},
        tx::run_transaction_effects,
    },
    event::{EventKind, EventPublisher, MessagePartCheckpointedEvent, PermissionRuleEvent},
    message::Message,
    permission::PersistedPermissionRule,
};

use super::{
    Session,
    cache::{SessionCache, SessionCachePolicy, SessionCacheStats},
    history::SessionHistoryStore,
    model::{SessionListRequest, SessionSummary},
};

pub(crate) struct SessionCommit {
    pub(crate) session: Session,
    pub(crate) touched_messages: Vec<Message>,
    pub(crate) client_events: Vec<EventKind>,
    pub(crate) persisted_rules: Vec<PersistedPermissionRule>,
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
