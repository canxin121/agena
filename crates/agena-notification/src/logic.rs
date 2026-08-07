//! 领域纯逻辑：渲染语义、排序分组、去重、过期、入口解析与恢复指令映射。
//!
//! 不依赖任何 UI 框架与运行时，全部可单测。

use crate::model::{
    Notification, NotificationKind, NotificationScope, NotificationSeverity, NotificationSurface,
};
use crate::service::NotificationError;
use agena_failure::RecoveryDirective;

/// 语义色（前端映射到主题）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SemanticColor {
    Neutral,
    Primary,
    Success,
    Warning,
    Danger,
}

/// 按严重级别映射语义色。
#[must_use]
pub fn severity_color(severity: NotificationSeverity) -> SemanticColor {
    match severity {
        NotificationSeverity::Info => SemanticColor::Primary,
        NotificationSeverity::Success => SemanticColor::Success,
        NotificationSeverity::Warning => SemanticColor::Warning,
        NotificationSeverity::Error => SemanticColor::Danger,
    }
}

/// 作用域的人类可读标签。
#[must_use]
pub fn scope_label(scope: &NotificationScope) -> String {
    match scope {
        NotificationScope::Global => "global".to_owned(),
        NotificationScope::Session(id) => format!("session:{id}"),
        NotificationScope::Workspace(id) => format!("workspace:{id}"),
        NotificationScope::ToolCall(id) => format!("tool:{id}"),
        NotificationScope::Provider(name) => format!("provider:{name}"),
        NotificationScope::Plugin(id) => format!("plugin:{id}"),
        NotificationScope::BackgroundTask(id) => format!("task:{id}"),
    }
}

/// 按主 surface 分组，组内按 priority 降序 + created_at 降序。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationGroup {
    pub surface: NotificationSurface,
    pub items: Vec<Notification>,
}

/// 分组并排序（稳定：先排序再分组）。
#[must_use]
pub fn sort_and_group(notifications: &[Notification]) -> Vec<NotificationGroup> {
    let mut sorted: Vec<&Notification> = notifications.iter().collect();
    sorted.sort_by(|a, b| {
        b.priority
            .cmp(&a.priority)
            .then_with(|| b.created_at_ms.cmp(&a.created_at_ms))
    });

    let mut groups: Vec<NotificationGroup> = Vec::new();
    for n in sorted {
        match groups.iter_mut().find(|g| g.surface == n.surface) {
            Some(g) => g.items.push(n.clone()),
            None => groups.push(NotificationGroup {
                surface: n.surface,
                items: vec![n.clone()],
            }),
        }
    }
    groups
}

/// 去重判定：dedup_key 相同视为重复；无 dedup_key 时按 kind + scope。
#[must_use]
pub fn should_dedupe(a: &Notification, b: &Notification) -> bool {
    match (&a.dedup_key, &b.dedup_key) {
        (Some(ka), Some(kb)) => ka == kb,
        _ => a.kind == b.kind && a.scope == b.scope,
    }
}

/// 过期判定（now_ms 大于等于 expires_at_ms）。
#[must_use]
pub fn is_expired(notification: &Notification, now_ms: i64) -> bool {
    notification.is_expired(now_ms)
}

/// 宿主默认主 surface（可按 kind/scope 覆盖）。
#[must_use]
pub fn default_surface(kind: &NotificationKind) -> NotificationSurface {
    match kind {
        NotificationKind::Notice { .. } => NotificationSurface::Banner,
        NotificationKind::Progress { .. } => NotificationSurface::Toast,
        NotificationKind::Status { .. } => NotificationSurface::StatusLine,
        NotificationKind::ModelStatus { .. } => NotificationSurface::ComposerChip,
        NotificationKind::PlanProgress { .. } => NotificationSurface::ComposerChip,
        NotificationKind::RunState { .. } => NotificationSurface::PlanPanel,
        NotificationKind::CommandExecution { .. } => NotificationSurface::ComposerFooter,
        NotificationKind::ToolCall { .. } => NotificationSurface::StatusLine,
        NotificationKind::BackgroundActivity { .. } => NotificationSurface::ActivitiesPanel,
        NotificationKind::PermissionRequest { .. } => NotificationSurface::PermissionDialog,
        NotificationKind::UserInputRequest { .. } => NotificationSurface::InputPrompt,
        NotificationKind::HistorySearch { .. } => NotificationSurface::HistorySearch,
        NotificationKind::TerminalTitle { .. } => NotificationSurface::TerminalTitle,
        NotificationKind::TerminalNotify { .. } => NotificationSurface::TerminalBell,
        NotificationKind::UsageUpdate { .. } => NotificationSurface::StatusLine,
        NotificationKind::Custom(_) => NotificationSurface::Toast,
    }
}

/// 解析入口：返回该入口对应的外部动作目标。
///
/// 执行不在领域层进行——宿主拿到 `ActionTarget` 后转交命令注册表 / 路由 / 剪贴板。
pub fn resolve_action_target(
    notification: &Notification,
    action_id: &str,
) -> Result<crate::model::ActionTarget, NotificationError> {
    notification
        .find_action(action_id)
        .map(|a| a.target.clone())
        .ok_or_else(|| NotificationError::NotFound(notification.id.clone()))
}

/// RecoveryDirective -> (label, command) 映射。
///
/// 替代 TUI `recovery_action` 命令映射，供宿主统一执行失败恢复指令。
/// `None` / `AskUser` 没有可自动执行的命令，返回 `None`。
#[must_use]
pub fn recovery_command(directive: RecoveryDirective) -> Option<(&'static str, &'static str)> {
    match directive {
        RecoveryDirective::Refresh => Some(("Refresh", "session.refresh")),
        RecoveryDirective::Reauthenticate => Some(("Sign in", "provider.authenticate")),
        RecoveryDirective::OpenSettings => Some(("Open settings", "settings.open")),
        RecoveryDirective::RequestPermission => Some(("Permissions", "permissions.open")),
        RecoveryDirective::Retry => Some(("Retry", "operation.retry")),
        RecoveryDirective::ChooseAlternative => Some(("Choose another", "alternative.choose")),
        RecoveryDirective::RestartPlugin => Some(("Restart plugin", "plugin.restart")),
        RecoveryDirective::RestartRuntime => Some(("Restart runtime", "runtime.restart")),
        RecoveryDirective::None | RecoveryDirective::AskUser => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        ActionTarget, Notification, NotificationAction, NotificationControl, NotificationKind,
        NotificationScope, NotificationSeverity, NotificationSource, NotificationSurface,
    };

    fn sample(
        id: &str,
        kind: NotificationKind,
        surface: NotificationSurface,
        priority: i32,
        created: i64,
    ) -> Notification {
        Notification {
            id: id.to_owned(),
            kind,
            severity: NotificationSeverity::Info,
            scope: NotificationScope::Global,
            surface,
            source: NotificationSource::App,
            summary: id.to_owned(),
            detail: None,
            control: NotificationControl::Dismiss,
            actions: Vec::new(),
            priority,
            dedup_key: None,
            created_at_ms: created,
            expires_at_ms: None,
            dismissed: false,
        }
    }

    #[test]
    fn severity_color_maps_errors_to_danger() {
        assert_eq!(
            severity_color(NotificationSeverity::Error),
            SemanticColor::Danger
        );
        assert_eq!(
            severity_color(NotificationSeverity::Success),
            SemanticColor::Success
        );
    }

    #[test]
    fn scope_label_formats_scopes() {
        assert_eq!(scope_label(&NotificationScope::Global), "global");
        assert_eq!(scope_label(&NotificationScope::Session(3)), "session:3");
        assert_eq!(
            scope_label(&NotificationScope::Plugin("x".into())),
            "plugin:x"
        );
    }

    #[test]
    fn sort_and_group_groups_by_surface_and_sorts_by_priority() {
        let n1 = sample(
            "a",
            NotificationKind::Notice { code: "x".into() },
            NotificationSurface::Banner,
            1,
            10,
        );
        let n2 = sample(
            "b",
            NotificationKind::Notice { code: "y".into() },
            NotificationSurface::Banner,
            5,
            20,
        );
        let n3 = sample(
            "c",
            NotificationKind::ToolCall {
                call_id: "c1".into(),
                name: "t".into(),
            },
            NotificationSurface::StatusLine,
            0,
            30,
        );
        let groups = sort_and_group(&[n1.clone(), n2.clone(), n3.clone()]);
        assert_eq!(groups.len(), 2);
        let banner = groups
            .iter()
            .find(|g| g.surface == NotificationSurface::Banner)
            .unwrap();
        assert_eq!(banner.items[0].id, "b"); // priority 5 first
        assert_eq!(banner.items[1].id, "a");
    }

    #[test]
    fn should_dedupe_uses_dedup_key_then_kind_scope() {
        let a = sample(
            "a",
            NotificationKind::Notice { code: "x".into() },
            NotificationSurface::Banner,
            0,
            0,
        );
        let b = sample(
            "b",
            NotificationKind::Notice { code: "x".into() },
            NotificationSurface::Banner,
            0,
            0,
        );
        assert!(should_dedupe(&a, &b)); // same kind+scope
        let c = sample(
            "c",
            NotificationKind::Notice { code: "z".into() },
            NotificationSurface::Banner,
            0,
            0,
        );
        assert!(!should_dedupe(&a, &c));
        let mut a2 = a.clone();
        a2.dedup_key = Some("k1".into());
        let mut b2 = b.clone();
        b2.dedup_key = Some("k1".into());
        assert!(should_dedupe(&a2, &b2));
        b2.dedup_key = Some("k2".into());
        assert!(!should_dedupe(&a2, &b2));
    }

    #[test]
    fn is_expired_checks_expires_at() {
        let n = sample(
            "a",
            NotificationKind::Notice { code: "x".into() },
            NotificationSurface::Banner,
            0,
            0,
        );
        assert!(!is_expired(&n, 1_000));
        let mut n2 = n.clone();
        n2.expires_at_ms = Some(100);
        assert!(is_expired(&n2, 100));
        assert!(!is_expired(&n2, 99));
    }

    #[test]
    fn default_surface_maps_kinds() {
        assert_eq!(
            default_surface(&NotificationKind::Progress {
                current: Some(1),
                total: Some(3)
            }),
            NotificationSurface::Toast
        );
        assert_eq!(
            default_surface(&NotificationKind::Status {
                state: crate::model::NotificationState::Running
            }),
            NotificationSurface::StatusLine
        );
        assert_eq!(
            default_surface(&NotificationKind::TerminalTitle { title: "t".into() }),
            NotificationSurface::TerminalTitle
        );
    }

    #[test]
    fn resolve_action_target_returns_target_or_not_found() {
        let mut n = sample(
            "a",
            NotificationKind::Notice { code: "x".into() },
            NotificationSurface::Banner,
            0,
            0,
        );
        n.actions = vec![NotificationAction {
            id: "go".into(),
            label: "Go".into(),
            target: ActionTarget::Navigate {
                route: "/settings".into(),
            },
        }];
        assert_eq!(
            resolve_action_target(&n, "go").unwrap(),
            ActionTarget::Navigate {
                route: "/settings".into()
            }
        );
        assert!(matches!(
            resolve_action_target(&n, "missing"),
            Err(NotificationError::NotFound(_))
        ));
    }

    #[test]
    fn recovery_command_maps_directives() {
        assert_eq!(
            recovery_command(RecoveryDirective::Refresh),
            Some(("Refresh", "session.refresh"))
        );
        assert_eq!(
            recovery_command(RecoveryDirective::Reauthenticate),
            Some(("Sign in", "provider.authenticate"))
        );
        assert_eq!(
            recovery_command(RecoveryDirective::Retry),
            Some(("Retry", "operation.retry"))
        );
        assert_eq!(recovery_command(RecoveryDirective::None), None);
        assert_eq!(recovery_command(RecoveryDirective::AskUser), None);
    }
}
