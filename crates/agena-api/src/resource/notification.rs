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
use agena_notification::service::NotificationFilter;

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

/// REST query filters for listing notifications (flat query-string shape, so
/// it works with `serde_urlencoded` / Axum `Query`).
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
#[allow(missing_docs)]
pub struct NotificationFilterParams {
    /// Scope variant: `global` | `session` | `workspace` | `tool_call` |
    /// `provider` | `plugin` | `background_task`. Omitted means global.
    pub scope_kind: Option<String>,
    /// Numeric scope key for `session` / `workspace`.
    pub scope_id: Option<i64>,
    /// Text scope key for `tool_call` / `provider` / `plugin` / `background_task`.
    pub scope_key: Option<String>,
    pub severity: Option<NotificationSeverity>,
    pub surface: Option<NotificationSurface>,
    pub source: Option<NotificationSource>,
    /// Defaults to true: only non-dismissed notifications.
    #[serde(default = "default_active_only")]
    pub active_only: bool,
    pub limit: Option<u64>,
    /// Cursor: return notifications with `created_at_ms < cursor` (descending pages).
    pub cursor: Option<i64>,
}

fn default_active_only() -> bool {
    true
}

impl Default for NotificationFilterParams {
    fn default() -> Self {
        Self {
            scope_kind: None,
            scope_id: None,
            scope_key: None,
            severity: None,
            surface: None,
            source: None,
            active_only: true,
            limit: None,
            cursor: None,
        }
    }
}

impl NotificationFilterParams {
    /// Convert into the domain filter used by the notification service.
    pub fn into_filter(self) -> Result<NotificationFilter, String> {
        let scope = match (
            self.scope_kind.as_deref(),
            self.scope_id,
            self.scope_key.as_deref(),
        ) {
            (None, _, _) => None,
            (Some("session"), Some(id), _) => Some(NotificationScope::Session(id)),
            (Some("workspace"), Some(id), _) => Some(NotificationScope::Workspace(id)),
            (Some("tool_call"), _, Some(key)) => Some(NotificationScope::ToolCall(key.to_owned())),
            (Some("provider"), _, Some(key)) => Some(NotificationScope::Provider(key.to_owned())),
            (Some("plugin"), _, Some(key)) => Some(NotificationScope::Plugin(key.to_owned())),
            (Some("background_task"), _, Some(key)) => {
                Some(NotificationScope::BackgroundTask(key.to_owned()))
            }
            (Some(kind), None, None) => {
                return Err(format!(
                    "scope_kind={kind} requires scope_id (session/workspace) or scope_key (tool_call/provider/plugin/background_task)"
                ));
            }
            (Some(other), _, _) => {
                return Err(format!("unsupported scope_kind: {other}"));
            }
        };
        Ok(NotificationFilter {
            scope,
            severity: self.severity,
            kind: None,
            surface: self.surface,
            source: self.source,
            active_only: self.active_only,
            limit: self.limit.map(|l| l as usize),
            cursor: self.cursor,
        })
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

#[cfg(test)]
mod contract_tests {
    use super::*;
    use agena_notification::model::{
        NotificationControl, NotificationKind, NotificationScope, NotificationSeverity,
        NotificationSource, NotificationSurface,
    };

    #[test]
    fn filter_params_default_to_active_only() {
        let params = NotificationFilterParams::default();
        assert!(params.active_only);
        let filter = params.into_filter().expect("default filter is valid");
        assert!(filter.active_only);
        assert_eq!(filter.scope, None);
    }

    #[test]
    fn filter_params_map_scope_variants() {
        let filter = NotificationFilterParams {
            scope_kind: Some("session".into()),
            scope_id: Some(7),
            ..Default::default()
        }
        .into_filter()
        .expect("session scope maps");
        assert_eq!(filter.scope, Some(NotificationScope::Session(7)));

        let filter = NotificationFilterParams {
            scope_kind: Some("plugin".into()),
            scope_key: Some("example.tools".into()),
            ..Default::default()
        }
        .into_filter()
        .expect("plugin scope maps");
        assert_eq!(
            filter.scope,
            Some(NotificationScope::Plugin("example.tools".into()))
        );

        assert!(
            NotificationFilterParams {
                scope_kind: Some("bogus".into()),
                ..Default::default()
            }
            .into_filter()
            .is_err()
        );
    }

    #[test]
    fn resource_projection_round_trips_domain_notification() {
        let notification = Notification {
            id: "n1".into(),
            kind: NotificationKind::Notice {
                code: "hello".into(),
            },
            severity: NotificationSeverity::Warning,
            scope: NotificationScope::Global,
            surface: NotificationSurface::Banner,
            source: NotificationSource::App,
            summary: "hello world".into(),
            detail: None,
            control: NotificationControl::Dismiss,
            actions: vec![NotificationAction {
                id: "go".into(),
                label: "Go".into(),
                target: ActionTarget::Copy {
                    text: "copy me".into(),
                },
            }],
            priority: 0,
            dedup_key: None,
            created_at_ms: 1_000,
            expires_at_ms: None,
            dismissed: false,
        };
        let resource = NotificationResource::from(&notification);
        let json = serde_json::to_value(&resource).expect("resource serializes");
        assert_eq!(json["summary"], "hello world");
        assert_eq!(json["severity"], "warning");
        assert_eq!(json["actions"][0]["target"]["target"], "copy");
        assert_eq!(json["actions"][0]["target"]["text"], "copy me");
    }

    #[test]
    fn stream_event_names_and_payloads_are_stable() {
        let notification = NotificationStreamEvent::Resumed { up_to_ms: 42 };
        assert_eq!(notification.event_name(), "resumed");
        assert_eq!(notification.payload()["up_to_ms"], 42);

        let lagged = NotificationStreamEvent::Lagged { skipped: 3 };
        assert_eq!(lagged.event_name(), "lagged");
        assert_eq!(lagged.payload()["skipped"], 3);

        let closed = NotificationStreamEvent::SubscriptionClosed {
            reason: "gone".into(),
        };
        assert_eq!(closed.event_name(), "subscription_closed");
        assert_eq!(closed.payload()["reason"], "gone");
    }
}
