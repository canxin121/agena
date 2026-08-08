//! 聚合转换：把现有运行时/领域事件转成统一 `Notification`。
//!
//! 转换是纯函数：输入事件 + 输出通知，便于单测。宿主按需调用并交给 store 落库/推送。

use agena_domain::{ActivityPayload, BackgroundActivity, BackgroundActivityStatus};
use agena_notification::model::{
    Notification, NotificationControl, NotificationKind, NotificationScope, NotificationSeverity,
    NotificationSource, NotificationSurface,
};
use agena_runtime_contracts::NoticePart;
use uuid::Uuid;

/// 由后台活动 id 生成稳定通知 id（便于按活动查询/去重）。
pub fn notification_id_for_activity(activity_id: &str) -> String {
    format!("notif_act_{activity_id}")
}

/// NoticePart -> Notice 通知。
///
/// 运行时核心产生的人类可见提示（如 `max_turns_exhausted`）统一进入通知总线。
pub fn from_notice_part(part: &NoticePart, scope: NotificationScope, now_ms: i64) -> Notification {
    let dedup_key = Some(format!("notice:{}", part.kind));
    Notification {
        id: format!("notif_notice_{}", Uuid::new_v4()),
        kind: NotificationKind::Notice {
            code: part.kind.clone(),
        },
        severity: NotificationSeverity::Info,
        scope,
        surface: NotificationSurface::Banner,
        source: NotificationSource::Runtime,
        summary: part.summary.clone(),
        detail: part.detail.clone(),
        control: NotificationControl::Dismiss,
        actions: Vec::new(),
        priority: 0,
        dedup_key,
        created_at_ms: now_ms,
        expires_at_ms: None,
        dismissed: false,
    }
}

/// ActivityPayload -> Option<Notification>。
///
/// 只把「用户可感知的状态变化」映射为通知：Notice 活动直接转成 Notice 通知；
/// 其余活动（资源、技能、推理、文本等）属于转录内容而非显示意图，返回 `None`。
pub fn from_activity_payload(
    payload: &ActivityPayload,
    scope: NotificationScope,
    now_ms: i64,
) -> Option<Notification> {
    match payload {
        ActivityPayload::Notice(notice) => Some(from_notice_part(
            &NoticePart {
                kind: notice.kind.clone(),
                summary: notice.summary.clone(),
                detail: notice.detail.clone(),
                title: notice.title.clone(),
            },
            scope,
            now_ms,
        )),
        _ => None,
    }
}

/// BackgroundActivity -> Notification。
///
/// 后台活动是通知的主要来源之一：每个活动状态变化（新建/运行/成功/失败/取消/停止）
/// 生成一条 `BackgroundActivity` 通知，落 ActivitiesPanel（可镜像 Log）。
///
/// 活动操作（stop/dismiss）不属于通知领域——它们走 activities REST API，
/// 因此这里不附加任何 `ActionTarget`。
pub fn from_background_activity(activity: &BackgroundActivity) -> Notification {
    let severity = match activity.status {
        BackgroundActivityStatus::Failed => NotificationSeverity::Error,
        BackgroundActivityStatus::Cancelled | BackgroundActivityStatus::Stopped => {
            NotificationSeverity::Warning
        }
        _ => NotificationSeverity::Info,
    };
    let scope = match activity.session_id {
        Some(session_id) => NotificationScope::Session(session_id),
        None => NotificationScope::BackgroundTask(activity.id.clone()),
    };
    let detail = activity
        .description
        .clone()
        .into_option()
        .or_else(|| activity.message.clone());
    Notification {
        id: notification_id_for_activity(&activity.id),
        kind: NotificationKind::BackgroundActivity {
            activity_id: activity.id.clone(),
        },
        severity,
        scope,
        surface: NotificationSurface::ActivitiesPanel,
        source: NotificationSource::Background,
        summary: activity.title.clone(),
        detail,
        control: NotificationControl::Dismiss,
        actions: Vec::new(),
        priority: match activity.status {
            BackgroundActivityStatus::Failed => 10,
            BackgroundActivityStatus::Succeeded => 1,
            _ => 0,
        },
        dedup_key: Some(format!("activity:{}", activity.id)),
        created_at_ms: activity.created_at_ms,
        expires_at_ms: None,
        dismissed: false,
    }
}

trait IntoOption {
    fn into_option(self) -> Option<String>;
}

impl IntoOption for String {
    fn into_option(self) -> Option<String> {
        (!self.is_empty()).then_some(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agena_domain::{BackgroundActivityKind, NoticeActivity};

    fn sample_activity(status: BackgroundActivityStatus) -> BackgroundActivity {
        BackgroundActivity {
            id: "task_1".to_owned(),
            kind: BackgroundActivityKind::Task,
            status,
            title: "Run tests".to_owned(),
            description: "cargo test".to_owned(),
            command: None,
            workdir: None,
            session_id: Some(7),
            parent_session_id: None,
            created_at_ms: 1000,
            started_at_ms: 1000,
            finished_at_ms: None,
            exit_code: None,
            message: None,
            failure: None,
            last_seq: 0,
            has_more: false,
            dropped_lines: 0,
            cancellable: true,
            dismissible: true,
        }
    }

    #[test]
    fn from_notice_part_maps_notice() {
        let part = NoticePart {
            kind: "max_turns_exhausted".into(),
            summary: "Turn budget exhausted".into(),
            detail: Some("Reduce scope".into()),
            title: None,
        };
        let n = from_notice_part(&part, NotificationScope::Session(1), 42);
        assert_eq!(
            n.kind,
            NotificationKind::Notice {
                code: "max_turns_exhausted".into()
            }
        );
        assert_eq!(n.source, NotificationSource::Runtime);
        assert_eq!(n.surface, NotificationSurface::Banner);
        assert_eq!(n.summary, "Turn budget exhausted");
    }

    #[test]
    fn from_activity_payload_notice_only() {
        let payload = ActivityPayload::Notice(NoticeActivity {
            kind: "warn".into(),
            summary: "hi".into(),
            detail: None,
            occurred_at_ms: None,
            title: None,
        });
        assert!(from_activity_payload(&payload, NotificationScope::Global, 1).is_some());
    }

    #[test]
    fn background_activity_failure_is_error() {
        let failed = from_background_activity(&sample_activity(BackgroundActivityStatus::Failed));
        assert_eq!(failed.severity, NotificationSeverity::Error);
        assert_eq!(
            failed.kind,
            NotificationKind::BackgroundActivity {
                activity_id: "task_1".into()
            }
        );
        assert_eq!(failed.surface, NotificationSurface::ActivitiesPanel);
        assert_eq!(failed.dedup_key.as_deref(), Some("activity:task_1"));
        assert!(failed.actions.is_empty()); // 活动操作不属于通知

        let running = from_background_activity(&sample_activity(BackgroundActivityStatus::Running));
        assert_eq!(running.severity, NotificationSeverity::Info);
    }
}
