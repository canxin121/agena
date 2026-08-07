//! Unified notification store, TUI emit sink, and action dispatcher (Phase 5).
//!
//! Replaces the TUI's bespoke `UiNotice` emit surface with the domain
//! `agena_notification::Notification` model. `flash_*` and failure notices now
//! push domain notifications into a `NotificationStore`; renderers (footer
//! banner, composer chips, toasts, activities) read from the same store;
//! notification action buttons are dispatched through `TuiActionDispatcher`.
//!
//! The runtime subscription (Phase 2 application wiring) will feed this store
//! later; until then the TUI's own emit helpers are the producers.

use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use agena_notification::{
    ActionTarget, Notification, NotificationAction, NotificationControl, NotificationKind,
    NotificationScope, NotificationSeverity, NotificationSource, NotificationSurface,
    recovery_command,
};

/// Wall-clock millis for notification lifecycle (created/expiry).
#[must_use]
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

/// Default flash lifetime, matching the former `DEFAULT_NOTICE_DURATION`.
pub const DEFAULT_NOTICE_TTL_MS: i64 = 5_000;

static NOTIFICATION_SEQ: AtomicU64 = AtomicU64::new(0);

fn next_id(prefix: &str) -> String {
    let seq = NOTIFICATION_SEQ.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}-{}-{seq}", now_ms())
}

/// In-memory notification list consumed by TUI renderers.
#[derive(Default)]
pub struct NotificationStore {
    notifications: Vec<Notification>,
    pinned: HashSet<String>,
}

// The subscription-fed accessors (toasts, composer_chips, activities, clear,
// all) and the action dispatcher are prepared for the Phase 2 runtime wiring
// and Phase 6 notification-action UI; only the footer banner consumes the
// store in Phase 5, so keep the rest available for those phases.
#[allow(dead_code)]
impl NotificationStore {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert or replace by id (newest emit wins).
    pub fn push(&mut self, notification: Notification) {
        if let Some(existing) = self
            .notifications
            .iter_mut()
            .find(|n| n.id == notification.id)
        {
            *existing = notification;
            return;
        }
        self.notifications.push(notification);
    }

    pub fn dismiss(&mut self, id: &str) {
        if let Some(existing) = self.notifications.iter_mut().find(|n| n.id == id) {
            existing.dismissed = true;
        }
        self.pinned.remove(id);
    }

    pub fn pin(&mut self, id: &str) {
        self.pinned.insert(id.to_owned());
    }

    pub fn clear(&mut self) {
        self.notifications.clear();
        self.pinned.clear();
    }

    /// Drop dismissed/expired notifications; pinned ones survive expiry.
    pub fn prune_expired(&mut self, now: i64) {
        self.notifications
            .retain(|n| !n.dismissed && (self.pinned.contains(&n.id) || !n.is_expired(now)));
    }

    #[must_use]
    pub fn all(&self) -> &[Notification] {
        &self.notifications
    }

    fn active<'a>(&'a self, now: i64) -> impl Iterator<Item = &'a Notification> + 'a {
        self.notifications
            .iter()
            .filter(move |n| !n.dismissed && (self.pinned.contains(&n.id) || !n.is_expired(now)))
    }

    /// Highest-priority active banner notification (footer row).
    #[must_use]
    pub fn banner(&self, now: i64) -> Option<&Notification> {
        self.active(now)
            .filter(|n| n.surface == NotificationSurface::Banner)
            .max_by(|a, b| {
                a.priority
                    .cmp(&b.priority)
                    .then_with(|| a.created_at_ms.cmp(&b.created_at_ms))
            })
    }

    /// Active toasts (floating overlays), newest first.
    #[must_use]
    pub fn toasts(&self, now: i64) -> Vec<&Notification> {
        let mut items: Vec<&Notification> = self
            .active(now)
            .filter(|n| n.surface == NotificationSurface::Toast)
            .collect();
        items.sort_by(|a, b| b.created_at_ms.cmp(&a.created_at_ms));
        items
    }

    /// Composer chip / status-line notifications, priority then newest.
    #[must_use]
    pub fn composer_chips(&self, now: i64) -> Vec<&Notification> {
        let mut items: Vec<&Notification> = self
            .active(now)
            .filter(|n| {
                matches!(
                    n.surface,
                    NotificationSurface::ComposerChip | NotificationSurface::StatusLine
                )
            })
            .collect();
        items.sort_by(|a, b| {
            b.priority
                .cmp(&a.priority)
                .then_with(|| b.created_at_ms.cmp(&a.created_at_ms))
        });
        items
    }

    /// Activities-panel notifications (background task scope), newest first.
    #[must_use]
    pub fn activities(&self, now: i64) -> Vec<&Notification> {
        let mut items: Vec<&Notification> = self
            .active(now)
            .filter(|n| {
                matches!(
                    n.surface,
                    NotificationSurface::ActivitiesPanel | NotificationSurface::BackgroundTask
                )
            })
            .collect();
        items.sort_by(|a, b| b.created_at_ms.cmp(&a.created_at_ms));
        items
    }
}

/// Build a plain notice notification (replaces `UiNotice::message`).
#[must_use]
pub fn notice_notification(
    severity: NotificationSeverity,
    summary: impl Into<String>,
    scope: NotificationScope,
    surface: NotificationSurface,
    detail: Option<String>,
    priority: i32,
) -> Notification {
    let created = now_ms();
    Notification {
        id: next_id("tui-notice"),
        kind: NotificationKind::Notice {
            code: "tui.flash".to_owned(),
        },
        severity,
        scope,
        surface,
        source: NotificationSource::App,
        summary: summary.into(),
        detail,
        control: NotificationControl::Dismiss,
        actions: Vec::new(),
        priority,
        dedup_key: None,
        created_at_ms: created,
        expires_at_ms: Some(created + DEFAULT_NOTICE_TTL_MS),
        dismissed: false,
    }
}

/// Build an error notification from a failure (replaces `UiNotice::from_failure`).
///
/// The recovery directive becomes a single `Recovery` action the dispatcher
/// translates through the command registry.
#[must_use]
pub fn failure_notification(
    failure: &agena_failure::Failure,
    scope: NotificationScope,
) -> Notification {
    let problem = agena_failure::UserProblem::from(failure);
    let mut notification = notice_notification(
        NotificationSeverity::Error,
        problem.user.fallback.clone(),
        scope,
        NotificationSurface::Banner,
        Some(problem.user.fallback.clone()),
        10,
    );
    if let Some((label, _)) = recovery_command(problem.recovery) {
        notification.actions.push(NotificationAction {
            id: "recovery".to_owned(),
            label: label.to_owned(),
            target: ActionTarget::Recovery(problem.recovery),
        });
    }
    notification
}

/// Translate a `Command` action's optional JSON input into slash-command args.
#[allow(dead_code)]
fn command_input_to_args(input: Option<&serde_json::Value>) -> String {
    match input {
        None => String::new(),
        Some(serde_json::Value::String(text)) => text.clone(),
        Some(value) => serde_json::to_string(value).unwrap_or_default(),
    }
}

/// Dispatches `NotificationAction` targets to the owning system:
/// Recovery -> command registry, Command -> command, Navigate -> route,
/// Copy -> clipboard. Reuses the existing `commands::CommandId` registry.
pub struct TuiActionDispatcher;

#[allow(dead_code)]
impl TuiActionDispatcher {
    pub fn dispatch(
        app: &mut crate::App,
        notification: &Notification,
        action: &NotificationAction,
    ) {
        let _ = notification;
        match &action.target {
            ActionTarget::Recovery(directive) => {
                if let Some((_, command)) = recovery_command(*directive) {
                    Self::run_command(app, command, "");
                }
            }
            ActionTarget::Command { command, input } => {
                let args = command_input_to_args(input.as_ref());
                Self::run_command(app, command, args.as_str());
            }
            ActionTarget::Navigate { route } => {
                let command = route.trim().trim_start_matches('/');
                Self::run_command(app, command, "");
            }
            ActionTarget::Copy { text } => {
                app.request_clipboard_copy(text.clone(), "Copied from notification.".to_owned());
            }
        }
    }

    fn run_command(app: &mut crate::App, command: &str, args: &str) {
        if let Some(spec) = crate::commands::find_command(command) {
            app.execute_command(spec, args);
        }
    }
}

/// Apply a local notification control (Dismiss/Pin) to the store.
///
/// `Copy` is handled by the caller where clipboard access lives; `Dismiss`
/// marks the notification dismissed, `Pin` keeps it alive across expiry.
#[allow(dead_code)]
pub fn apply_control(store: &mut NotificationStore, id: &str, control: NotificationControl) {
    match control {
        NotificationControl::Dismiss => store.dismiss(id),
        NotificationControl::Pin => store.pin(id),
        NotificationControl::Copy => {}
    }
}

#[cfg(test)]
mod tests {
    use super::{DEFAULT_NOTICE_TTL_MS, NotificationStore, notice_notification, now_ms};
    use agena_notification::{NotificationScope, NotificationSeverity, NotificationSurface};

    fn sample(surface: NotificationSurface, priority: i32, offset_ms: i64) -> super::Notification {
        notice_notification(
            NotificationSeverity::Info,
            "sample",
            NotificationScope::Global,
            surface,
            None,
            priority,
        )
        .tap_offset(offset_ms)
    }

    trait TapOffset {
        fn tap_offset(self, offset_ms: i64) -> Self;
    }

    impl TapOffset for super::Notification {
        fn tap_offset(mut self, offset_ms: i64) -> Self {
            self.created_at_ms = now_ms() + offset_ms;
            self
        }
    }

    #[test]
    fn banner_prefers_highest_priority_then_newest() {
        let mut store = NotificationStore::new();
        store.push(sample(NotificationSurface::Banner, 1, 0));
        store.push(sample(NotificationSurface::Banner, 5, 100));
        store.push(sample(NotificationSurface::Toast, 9, 200));
        let banner = store.banner(now_ms()).expect("banner");
        assert_eq!(banner.priority, 5);
    }

    #[test]
    fn expired_and_dismissed_are_hidden_but_pinned_survive() {
        let mut store = NotificationStore::new();
        let id = "x";
        let mut notification = sample(NotificationSurface::Banner, 1, 0);
        notification.id = id.to_owned();
        store.push(notification);
        store.pin(id);
        store.prune_expired(now_ms() + DEFAULT_NOTICE_TTL_MS + 1);
        assert!(store.banner(now_ms() + DEFAULT_NOTICE_TTL_MS + 1).is_some());
        store.dismiss(id);
        assert!(store.banner(now_ms()).is_none());
    }
}
