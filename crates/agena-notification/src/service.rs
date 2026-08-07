//! 通知服务端口（应用层唯一入口）。
//!
//! 领域层只定义契约与解析语义，不包含任何执行器：
//! - `emit` / `list` / `dismiss` / `resolve_target` 由实现方（如 agena-runtime-notifications）提供；
//! - `resolve_target` 返回外部动作目标，真正执行交给宿主命令执行器。

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::model::{
    ActionTarget, Notification, NotificationControl, NotificationId, NotificationKind,
    NotificationScope, NotificationSeverity, NotificationSource, NotificationSurface,
};

/// 通知服务错误。
#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum NotificationError {
    #[error("notification not found: {0}")]
    NotFound(NotificationId),
    #[error("invalid notification request: {0}")]
    Validation(String),
    #[error("notification conflict: {0}")]
    Conflict(String),
    #[error("notification service unavailable: {0}")]
    Unavailable(String),
}

/// 查询过滤器（分页游标基于 created_at_ms）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct NotificationFilter {
    pub scope: Option<NotificationScope>,
    pub severity: Option<NotificationSeverity>,
    pub kind: Option<NotificationKind>,
    pub surface: Option<NotificationSurface>,
    pub source: Option<NotificationSource>,
    /// 默认只返回未忽略（active）通知。
    #[serde(default = "default_true")]
    pub active_only: bool,
    pub limit: Option<usize>,
    /// 游标：返回 created_at_ms < cursor 的条目（倒序分页）。
    pub cursor: Option<i64>,
}

fn default_true() -> bool {
    true
}

/// 发出通知的请求。
///
/// 注：请求携带 `NotificationAction`（其 `ActionTarget` 承载 `agena-failure::RecoveryDirective`，
/// 无 `JsonSchema`），因此本类型只实现 serde 契约；API 层 resource 负责补充 JsonSchema。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmitNotificationRequest {
    pub kind: NotificationKind,
    pub severity: NotificationSeverity,
    pub scope: NotificationScope,
    #[serde(default)]
    pub source: NotificationSource,
    /// 缺省时由宿主按 kind 分配主 surface（见 `logic::default_surface`）。
    pub surface: Option<NotificationSurface>,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default)]
    pub control: NotificationControl,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<crate::model::NotificationAction>,
    #[serde(default)]
    pub priority: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dedup_key: Option<String>,
    /// 相对过期时长（毫秒）；缺省为不过期。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ttl_ms: Option<i64>,
}

/// 订阅句柄：实现方提供推送流。
#[async_trait]
pub trait NotificationSubscription: Send + Sync {
    fn filter(&self) -> &NotificationFilter;
    /// 拉取下一条通知；流结束时返回 None。
    async fn next_notification(&mut self) -> Option<Notification>;
}

/// 通知服务端口（应用层唯一入口）。
#[async_trait]
pub trait NotificationService: Send + Sync {
    /// 发出通知（聚合/去重/持久化/推送订阅者）。
    async fn emit(&self, request: EmitNotificationRequest) -> Result<Notification, NotificationError>;
    /// 按过滤器查询（分页）。
    async fn list(&self, filter: NotificationFilter) -> Result<Vec<Notification>, NotificationError>;
    /// 忽略/关闭。
    async fn dismiss(&self, id: NotificationId, reason: Option<String>) -> Result<(), NotificationError>;
    /// 解析并返回入口对应的外部动作目标（执行交给宿主命令执行器）。
    async fn resolve_target(&self, id: NotificationId, action_id: String)
        -> Result<ActionTarget, NotificationError>;
    /// 订阅推送（SSE 后端等）。
    fn subscribe(&self, filter: NotificationFilter) -> Box<dyn NotificationSubscription>;
}
