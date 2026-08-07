//! 内存通知存储：实现 [`NotificationService`]，支持去重、过期清理、容量上限、
//! broadcast 订阅（带 lagged 检测，供 SSE 等需要滞后信号的消费端使用）。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use agena_notification::logic::{default_surface, resolve_action_target};
use agena_notification::model::{ActionTarget, Notification, NotificationId};
use agena_notification::service::{
    EmitNotificationRequest, NotificationError, NotificationFilter, NotificationService,
    NotificationSubscription,
};
use async_trait::async_trait;
use tokio::sync::broadcast;
use uuid::Uuid;

/// 当前时间（毫秒）。
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// 订阅事件：通知、滞后信号、流关闭。
#[derive(Debug, Clone)]
pub enum SubscriptionEvent {
    Notification(Notification),
    Lagged(u64),
    Closed,
}

/// 判断通知是否匹配过滤器。
pub fn filter_matches(filter: &NotificationFilter, n: &Notification) -> bool {
    if let Some(scope) = &filter.scope {
        if &n.scope != scope {
            return false;
        }
    }
    if let Some(severity) = filter.severity {
        if n.severity != severity {
            return false;
        }
    }
    if let Some(kind) = &filter.kind {
        if &n.kind != kind {
            return false;
        }
    }
    if let Some(surface) = filter.surface {
        if n.surface != surface {
            return false;
        }
    }
    if let Some(source) = filter.source {
        if n.source != source {
            return false;
        }
    }
    if filter.active_only && n.dismissed {
        return false;
    }
    true
}

#[derive(Default)]
struct StoreInner {
    notifications: Vec<Notification>,
    by_id: HashMap<NotificationId, usize>,
}

/// 内存通知存储。
///
/// - 去重：`ingest`/`emit` 时若 `dedup_key` 已存在则替换旧条目；若 `id` 已存在也替换。
/// - 容量：超过 `capacity` 时淘汰 `created_at_ms` 最旧的条目。
/// - 过期：`prune_expired(now_ms)` 清除已过期通知。
#[derive(Clone)]
pub struct InMemoryNotificationStore {
    inner: Arc<Mutex<StoreInner>>,
    tx: broadcast::Sender<SubscriptionEvent>,
    capacity: usize,
}

impl InMemoryNotificationStore {
    pub fn new(capacity: usize) -> Self {
        let (tx, _rx) = broadcast::channel(1024);
        Self {
            inner: Arc::new(Mutex::new(StoreInner::default())),
            tx,
            capacity: capacity.max(1),
        }
    }

    /// 订阅原始事件流（含 Lagged/Closed 信号），供 SSE 等消费端使用。
    pub fn subscribe_events(&self) -> broadcast::Receiver<SubscriptionEvent> {
        self.tx.subscribe()
    }

    /// 直接入库一条已构造的通知（聚合器入口）。返回入库后的通知。
    pub fn ingest(&self, notification: Notification) -> Notification {
        let mut inner = self.inner.lock().expect("store lock poisoned");
        let pos = Self::upsert_locked(&mut inner, notification, self.capacity);
        let stored = inner.notifications[pos].clone();
        let _ = self
            .tx
            .send(SubscriptionEvent::Notification(stored.clone()));
        stored
    }

    /// 清除已过期通知，返回清除数量。
    pub fn prune_expired(&self, now: i64) -> usize {
        let mut inner = self.inner.lock().expect("store lock poisoned");
        let before = inner.notifications.len();
        inner.notifications.retain(|n| !n.is_expired(now));
        inner.by_id = inner
            .notifications
            .iter()
            .enumerate()
            .map(|(i, n)| (n.id.clone(), i))
            .collect();
        before - inner.notifications.len()
    }

    fn upsert_locked(inner: &mut StoreInner, notification: Notification, capacity: usize) -> usize {
        // id 冲突则替换
        if let Some(&pos) = inner.by_id.get(&notification.id) {
            inner.notifications[pos] = notification;
            return pos;
        }
        // dedup_key 冲突则替换旧条目（保持新条目的 id）
        if let Some(dk) = &notification.dedup_key {
            if let Some(pos) = inner
                .notifications
                .iter()
                .position(|n| n.dedup_key.as_deref() == Some(dk))
            {
                let old_id = inner.notifications[pos].id.clone();
                let new_id = notification.id.clone();
                inner.by_id.remove(&old_id);
                inner.notifications[pos] = notification;
                inner.by_id.insert(new_id, pos);
                return pos;
            }
        }
        // 容量淘汰最旧
        if inner.notifications.len() >= capacity {
            if let Some(oldest_pos) = inner
                .notifications
                .iter()
                .enumerate()
                .min_by_key(|(_, n)| n.created_at_ms)
                .map(|(i, _)| i)
            {
                let removed_id = inner.notifications.remove(oldest_pos).id;
                inner.by_id.remove(&removed_id);
            }
        }
        inner
            .by_id
            .insert(notification.id.clone(), inner.notifications.len());
        inner.notifications.push(notification);
        inner.notifications.len() - 1
    }
}

#[async_trait]
impl NotificationService for InMemoryNotificationStore {
    async fn emit(
        &self,
        request: EmitNotificationRequest,
    ) -> Result<Notification, NotificationError> {
        let id = format!("notif_{}", Uuid::new_v4());
        let now = now_ms();
        let surface = request
            .surface
            .unwrap_or_else(|| default_surface(&request.kind));
        let notification = Notification {
            id,
            kind: request.kind,
            severity: request.severity,
            scope: request.scope,
            surface,
            source: request.source,
            summary: request.summary,
            detail: request.detail,
            control: request.control,
            actions: request.actions,
            priority: request.priority,
            dedup_key: request.dedup_key,
            created_at_ms: now,
            expires_at_ms: request.ttl_ms.map(|ttl| now + ttl),
            dismissed: false,
        };
        Ok(self.ingest(notification))
    }

    async fn list(
        &self,
        filter: NotificationFilter,
    ) -> Result<Vec<Notification>, NotificationError> {
        let inner = self.inner.lock().expect("store lock poisoned");
        let mut matched: Vec<&Notification> = inner
            .notifications
            .iter()
            .filter(|n| filter_matches(&filter, n))
            .filter(|n| filter.cursor.map_or(true, |c| n.created_at_ms < c))
            .collect();
        matched.sort_by(|a, b| b.created_at_ms.cmp(&a.created_at_ms));
        let limit = filter.limit.unwrap_or(usize::MAX).min(matched.len());
        Ok(matched.into_iter().take(limit).cloned().collect())
    }

    async fn dismiss(
        &self,
        id: NotificationId,
        _reason: Option<String>,
    ) -> Result<(), NotificationError> {
        let mut inner = self.inner.lock().expect("store lock poisoned");
        let pos = inner
            .by_id
            .get(&id)
            .copied()
            .ok_or_else(|| NotificationError::NotFound(id.clone()))?;
        inner.notifications[pos].dismissed = true;
        let updated = inner.notifications[pos].clone();
        drop(inner);
        let _ = self.tx.send(SubscriptionEvent::Notification(updated));
        Ok(())
    }

    async fn resolve_target(
        &self,
        id: NotificationId,
        action_id: String,
    ) -> Result<ActionTarget, NotificationError> {
        let inner = self.inner.lock().expect("store lock poisoned");
        let notification = inner
            .by_id
            .get(&id)
            .and_then(|&pos| inner.notifications.get(pos))
            .ok_or_else(|| NotificationError::NotFound(id.clone()))?;
        resolve_action_target(notification, &action_id)
    }

    fn subscribe(&self, filter: NotificationFilter) -> Box<dyn NotificationSubscription> {
        Box::new(BroadcastSubscription {
            rx: self.tx.subscribe(),
            filter,
        })
    }
}

/// 基于 broadcast 的领域订阅实现。
///
/// `next_notification` 只返回匹配过滤器的通知；收到 Lagged 或流关闭时返回 `None`
/// （领域 trait 无法表达滞后细节；需要滞后信号的消费端请用 `subscribe_events`）。
pub struct BroadcastSubscription {
    rx: broadcast::Receiver<SubscriptionEvent>,
    filter: NotificationFilter,
}

#[async_trait]
impl NotificationSubscription for BroadcastSubscription {
    fn filter(&self) -> &NotificationFilter {
        &self.filter
    }

    async fn next_notification(&mut self) -> Option<Notification> {
        loop {
            match self.rx.recv().await {
                Ok(SubscriptionEvent::Notification(n)) if filter_matches(&self.filter, &n) => {
                    return Some(n);
                }
                Ok(SubscriptionEvent::Notification(_)) => continue,
                Ok(SubscriptionEvent::Lagged(_)) | Ok(SubscriptionEvent::Closed) => return None,
                Err(broadcast::error::RecvError::Lagged(_))
                | Err(broadcast::error::RecvError::Closed) => {
                    return None;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agena_notification::model::{
        ActionTarget, NotificationAction, NotificationControl, NotificationKind, NotificationScope,
        NotificationSeverity, NotificationSource,
    };
    use agena_notification::service::NotificationFilter;

    fn emit_request(
        summary: &str,
        dedup: Option<&str>,
        ttl: Option<i64>,
    ) -> EmitNotificationRequest {
        EmitNotificationRequest {
            kind: NotificationKind::Notice {
                code: "test".into(),
            },
            severity: NotificationSeverity::Info,
            scope: NotificationScope::Global,
            source: NotificationSource::App,
            surface: None,
            summary: summary.to_owned(),
            detail: None,
            control: NotificationControl::Dismiss,
            actions: Vec::new(),
            priority: 0,
            dedup_key: dedup.map(str::to_owned),
            ttl_ms: ttl,
        }
    }

    fn make_notification(id: &str, summary: &str, created: i64) -> Notification {
        Notification {
            id: id.to_owned(),
            kind: NotificationKind::Notice {
                code: "test".into(),
            },
            severity: NotificationSeverity::Info,
            scope: NotificationScope::Global,
            surface: agena_notification::model::NotificationSurface::Banner,
            source: NotificationSource::App,
            summary: summary.to_owned(),
            detail: None,
            control: NotificationControl::Dismiss,
            actions: Vec::new(),
            priority: 0,
            dedup_key: None,
            created_at_ms: created,
            expires_at_ms: None,
            dismissed: false,
        }
    }

    #[tokio::test]
    async fn emit_defaults_surface_and_expiry() {
        let store = InMemoryNotificationStore::new(64);
        let n = store
            .emit(emit_request("hi", None, Some(5000)))
            .await
            .unwrap();
        assert_eq!(
            n.surface,
            agena_notification::model::NotificationSurface::Banner
        );
        assert!(n.expires_at_ms.is_some());
        assert!(n.created_at_ms > 0);
    }

    #[tokio::test]
    async fn dedup_replaces_same_dedup_key() {
        let store = InMemoryNotificationStore::new(64);
        let a = store
            .emit(emit_request("first", Some("k1"), None))
            .await
            .unwrap();
        let b = store
            .emit(emit_request("second", Some("k1"), None))
            .await
            .unwrap();
        assert_ne!(a.id, b.id);
        let list = store.list(NotificationFilter::default()).await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].summary, "second");
    }

    #[tokio::test]
    async fn list_filters_and_paginates() {
        let store = InMemoryNotificationStore::new(64);
        store.ingest(make_notification("n1", "a", 1_000));
        store.ingest(make_notification("n2", "b", 2_000));
        let list = store.list(NotificationFilter::default()).await.unwrap();
        assert_eq!(list.len(), 2);
        // 倒序：最新在前
        assert_eq!(list[0].summary, "b");
        // 游标分页
        let filter = NotificationFilter {
            cursor: Some(2_000),
            ..Default::default()
        };
        let page = store.list(filter).await.unwrap();
        assert_eq!(page.len(), 1);
        assert_eq!(page[0].summary, "a");
    }

    #[tokio::test]
    async fn dismiss_marks_and_hides_from_active_list() {
        let store = InMemoryNotificationStore::new(64);
        let n = store.emit(emit_request("x", None, None)).await.unwrap();
        store.dismiss(n.id.clone(), None).await.unwrap();
        let active = store.list(NotificationFilter::default()).await.unwrap();
        assert!(active.is_empty());
        let all = store
            .list(NotificationFilter {
                active_only: false,
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(all.len(), 1);
        assert!(all[0].dismissed);
    }

    #[tokio::test]
    async fn resolve_target_returns_action_or_not_found() {
        let store = InMemoryNotificationStore::new(64);
        let mut req = emit_request("x", None, None);
        req.actions = vec![NotificationAction {
            id: "go".into(),
            label: "Go".into(),
            target: ActionTarget::Navigate {
                route: "/settings".into(),
            },
        }];
        let n = store.emit(req).await.unwrap();
        assert_eq!(
            store
                .resolve_target(n.id.clone(), "go".into())
                .await
                .unwrap(),
            ActionTarget::Navigate {
                route: "/settings".into()
            }
        );
        assert!(matches!(
            store.resolve_target(n.id, "nope".into()).await,
            Err(NotificationError::NotFound(_))
        ));
    }

    #[test]
    fn prune_expired_removes_expired() {
        let store = InMemoryNotificationStore::new(64);
        let now = now_ms();
        let mut n = Notification {
            id: "n1".into(),
            kind: NotificationKind::Notice { code: "x".into() },
            severity: NotificationSeverity::Info,
            scope: NotificationScope::Global,
            surface: agena_notification::model::NotificationSurface::Banner,
            source: NotificationSource::App,
            summary: "x".into(),
            detail: None,
            control: NotificationControl::Dismiss,
            actions: Vec::new(),
            priority: 0,
            dedup_key: None,
            created_at_ms: now,
            expires_at_ms: Some(now + 100),
            dismissed: false,
        };
        store.ingest(n.clone());
        assert_eq!(store.prune_expired(now + 50), 0);
        n.id = "n2".into();
        n.expires_at_ms = Some(now + 300);
        store.ingest(n);
        assert_eq!(store.prune_expired(now + 200), 1);
    }

    #[tokio::test]
    async fn capacity_evicts_oldest() {
        let store = InMemoryNotificationStore::new(2);
        store.emit(emit_request("a", None, None)).await.unwrap();
        store.emit(emit_request("b", None, None)).await.unwrap();
        store.emit(emit_request("c", None, None)).await.unwrap();
        let list = store.list(NotificationFilter::default()).await.unwrap();
        assert_eq!(list.len(), 2);
        assert!(list.iter().all(|n| n.summary != "a"));
    }

    #[tokio::test]
    async fn subscription_delivers_matching_notifications() {
        let store = InMemoryNotificationStore::new(64);
        let mut sub = store.subscribe(NotificationFilter::default());
        store.emit(emit_request("s1", None, None)).await.unwrap();
        let got = sub.next_notification().await.unwrap();
        assert_eq!(got.summary, "s1");
    }
}
