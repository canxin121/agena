//! Wire resources for the unified notification service (Phase 3).
//!
//! The domain model in `agena-notification` is the single source of truth for
//! notification content; these types project it onto the stable wire contract
//! consumed by REST and SSE clients.

use serde::{Deserialize, Serialize};

use agena_notification::model::{
    ActionTarget, Notification, NotificationAction, NotificationControl, NotificationKind,
    NotificationScope, NotificationSeverity, NotificationSource, NotificationSurface,
};

/// Wire projection of one unified notification.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct NotificationResource {
    pub id: String,
    pub kind: NotificationKind,
    pub severity: NotificationSeverity,
    pub scope: NotificationScope,
    pub surface: NotificationSurface,
    pub source: NotificationSource,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    pub control: NotificationControl,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<NotificationActionResource>,
    pub priority: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dedup_key: Option<String>,
    pub created_at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at_ms: Option<i64>,
    #[serde(default)]
    pub dismissed: bool,
}

impl From<&Notification> for NotificationResource {
    fn from(n: &Notification) -> Self {
        Self {
            id: n.id.clone(),
            kind: n.kind.clone(),
            severity: n.severity,
            scope: n.scope.clone(),
            surface: n.surface,
            source: n.source,
            summary: n.summary.clone(),
            detail: n.detail.clone(),
            control: n.control,
            actions: n
                .actions
                .iter()
                .map(NotificationActionResource::from)
                .collect(),
            priority: n.priority,
            dedup_key: n.dedup_key.clone(),
            created_at_ms: n.created_at_ms,
            expires_at_ms: n.expires_at_ms,
            dismissed: n.dismissed,
        }
    }
}

/// Wire shape of one actionable button on a notification.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct NotificationActionResource {
    pub id: String,
    pub label: String,
    pub target: NotificationActionTargetResource,
}

impl From<&NotificationAction> for NotificationActionResource {
    fn from(a: &NotificationAction) -> Self {
        Self {
            id: a.id.clone(),
            label: a.label.clone(),
            target: NotificationActionTargetResource::from(&a.target),
        }
    }
}

/// Wire shape of a resolved action target. The domain `ActionTarget` cannot
/// derive `JsonSchema` (it embeds `agena-failure::RecoveryDirective`), so the
/// API owns this typed projection.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "target", rename_all = "snake_case")]
pub enum NotificationActionTargetResource {
    /// Failure-recovery directive; the host translates it into a command.
    Recovery { directive: String },
    /// Generic command (application command registry / plugin command).
    Command {
        command: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        input: Option<serde_json::Value>,
    },
    /// Frontend route (including open_url).
    Navigate { route: String },
    /// Copy text to the clipboard.
    Copy { text: String },
}

impl From<&ActionTarget> for NotificationActionTargetResource {
    fn from(target: &ActionTarget) -> Self {
        match target {
            ActionTarget::Recovery(directive) => Self::Recovery {
                directive: recovery_directive_name(directive).to_owned(),
            },
            ActionTarget::Command { command, input } => Self::Command {
                command: command.clone(),
                input: input.clone(),
            },
            ActionTarget::Navigate { route } => Self::Navigate {
                route: route.clone(),
            },
            ActionTarget::Copy { text } => Self::Copy { text: text.clone() },
        }
    }
}

fn recovery_directive_name(directive: &agena_failure::RecoveryDirective) -> &'static str {
    use agena_failure::RecoveryDirective::*;
    match directive {
        None => "none",
        Refresh => "refresh",
        Reauthenticate => "reauthenticate",
        OpenSettings => "open_settings",
        RequestPermission => "request_permission",
        AskUser => "ask_user",
        Retry => "retry",
        ChooseAlternative => "choose_alternative",
        RestartPlugin => "restart_plugin",
        RestartRuntime => "restart_runtime",
    }
}

/// Server → client messages on the notification SSE stream.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum NotificationStreamEvent {
    /// A new or updated notification.
    Notification(Box<NotificationResource>),
    /// The broadcast channel dropped `skipped` messages; the client should
    /// backfill with a fresh list request.
    Lagged { skipped: u64 },
    /// History replay finished; live events continue from `up_to_ms`.
    Resumed { up_to_ms: i64 },
    /// Server closed the stream.
    SubscriptionClosed { reason: String },
}

impl NotificationStreamEvent {
    /// SSE `event:` name for this message.
    pub fn event_name(&self) -> &'static str {
        match self {
            Self::Notification(_) => "notification",
            Self::Lagged { .. } => "lagged",
            Self::Resumed { .. } => "resumed",
            Self::SubscriptionClosed { .. } => "subscription_closed",
        }
    }

    /// JSON `data:` payload for this message.
    pub fn payload(&self) -> serde_json::Value {
        match self {
            Self::Notification(n) => match serde_json::to_value(n) {
                Ok(payload) => payload,
                Err(error) => {
                    tracing::error!(
                        notification_id = %n.id,
                        diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                            "serialize a notification stream payload",
                            &error,
                        ),
                        "notification stream payload could not be encoded"
                    );
                    serde_json::json!({
                        "projection_error": "The notification payload could not be encoded."
                    })
                }
            },
            Self::Lagged { skipped } => serde_json::json!({ "skipped": skipped }),
            Self::Resumed { up_to_ms } => serde_json::json!({ "up_to_ms": up_to_ms }),
            Self::SubscriptionClosed { reason } => serde_json::json!({ "reason": reason }),
        }
    }
}
