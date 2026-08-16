//! Terminal-integration presentation projection and dispatch.
//!
//! This module owns the *concrete* title text and notification summaries that
//! depend on the active session, activity, and user locale. It projects those
//! into OSC frames (via `agena_tui_platform::terminal::integration`) and emits
//! them through `TerminalRuntime`. It has no session or application logic of
//! its own — callers (the run loop and the session event handlers) supply the
//! presentation inputs.

use crate::{App, SessionActivity, TerminalRuntime};
use agena_tui_platform::terminal::integration::{
    NotificationMethod, ProgressState, notification_frames, notification_method, progress_frames,
    title_frames,
};

use agena_tui::presentation_config::TerminalIntegrationMode;

/// Whether the terminal currently supports window-title frames. Writes are
/// still attempted when the capability is unknown so a conservative profile
/// that did not prove support does not permanently disable titles. The
/// user-facing mode overrides the capability evidence.
fn window_title_operational(app: &App) -> bool {
    if app.launch.tui_config.terminal_title == TerminalIntegrationMode::Disabled {
        return false;
    }
    if app.launch.tui_config.terminal_title == TerminalIntegrationMode::Enabled {
        return true;
    }
    app.launch.terminal_context.as_ref().is_some_and(|context| {
        context.capabilities.window_title.is_operational()
            || matches!(
                context.capabilities.window_title.support,
                agena_tui::terminal_capabilities::Support::Unknown
            )
    })
}

/// Whether the terminal currently supports attention notifications. The
/// user-facing mode overrides the capability evidence.
fn notifications_operational(app: &App) -> bool {
    if app.launch.tui_config.terminal_notifications == TerminalIntegrationMode::Disabled {
        return false;
    }
    if app.launch.tui_config.terminal_notifications == TerminalIntegrationMode::Enabled {
        return true;
    }
    app.launch
        .terminal_context
        .as_ref()
        .is_some_and(|context| context.capabilities.terminal_notifications.is_operational())
}

/// The activity projected to the terminal title and progress bar.
///
/// This is the single unified projection: the App's `current_session_activity`
/// derives from the server's authoritative session state machine
/// (`SessionResource.state`), with pending-interactive requests refining what
/// an awaiting state is for. The bundled `agena.terminal` plugin also publishes
/// a hook-driven `agena.terminal.activity` display segment, but hooks only
/// observe idle/running/blocked and can lag the server lease, so it is *not*
/// consulted here — the state machine is the source of truth.
fn current_title_text(app: &App) -> String {
    let session_title = app.current_or_selected_session_title();
    let workspace = crate::app_backend::plugin_effects::workspace_name(&app.application);
    let activity = app.current_session_activity();
    let state = match activity {
        SessionActivity::Idle => None,
        SessionActivity::Running => Some(app.i18n.text("terminal-title-working")),
        SessionActivity::AwaitingPermission => Some(app.i18n.text("terminal-title-permission")),
        SessionActivity::AwaitingUserInput => Some(app.i18n.text("terminal-title-user-input")),
        SessionActivity::Blocked => Some(app.i18n.text("terminal-title-blocked")),
    };
    // Terminals without a native OSC 9;4 progress indicator rely on the
    // title alone to show activity, so the state text leads the title where
    // it cannot be truncated away. Terminals with native progress keep the
    // state as a trailing suffix because the indicator is the primary cue.
    let progress_native = progress_operational(app);
    match (session_title, state) {
        (Some(title), Some(state)) if progress_native => format!("{title} · {state}"),
        (Some(title), Some(state)) => format!("{state} · {title}"),
        (Some(title), None) => title,
        (None, Some(state)) if progress_native => format!("{} · {state}", workspace),
        (None, Some(state)) => format!("{state} · {workspace}"),
        (None, None) => workspace,
    }
}

/// Computes the window-title frames for the current presentation state, or
/// `None` when the title has not changed since the last emission.
pub(crate) fn title_frames_if_changed(app: &App) -> Option<Vec<Vec<u8>>> {
    if !window_title_operational(app) {
        return None;
    }
    let title = current_title_text(app);
    if app
        .terminal_integration
        .last_title()
        .is_some_and(|last| last == title)
        && !app.terminal_integration.title_due()
    {
        return None;
    }
    let family = app
        .launch
        .terminal_context
        .as_ref()
        .map(|context| context.identity.family)
        .unwrap_or(agena_tui::terminal::TerminalFamily::Unknown);
    Some(title_frames(family, &title))
}

/// Emits the title frames for the current state, if any changed.
pub(crate) fn sync_terminal_title(
    app: &mut App,
    terminal: &mut TerminalRuntime,
) -> crate::Result<()> {
    let Some(frames) = title_frames_if_changed(app) else {
        return Ok(());
    };
    let owned_frames = frames
        .iter()
        .map(|frame| frame.as_slice())
        .collect::<Vec<_>>();
    terminal.write_protocol_frames(&owned_frames)?;
    let title = current_title_text(app);
    app.terminal_integration.note_title_emitted(title);
    Ok(())
}

/// Whether the terminal currently supports OSC 9;4 native progress
/// reporting. The user-facing mode overrides the capability evidence; like
/// notifications, the sequence is not emitted for unsupported endpoints.
fn progress_operational(app: &App) -> bool {
    if app.launch.tui_config.terminal_progress == TerminalIntegrationMode::Disabled {
        return false;
    }
    if app.launch.tui_config.terminal_progress == TerminalIntegrationMode::Enabled {
        return true;
    }
    app.launch
        .terminal_context
        .as_ref()
        .is_some_and(|context| context.capabilities.terminal_progress.is_operational())
}

/// The OSC 9;4 progress state for the current activity. `Idle` clears the
/// indicator; interactive waits map to the paused/warning state and a
/// blocked run to the error state.
fn current_progress_state(app: &App) -> ProgressState {
    match app.current_session_activity() {
        SessionActivity::Idle => ProgressState::Clear,
        SessionActivity::Running => ProgressState::Working,
        SessionActivity::AwaitingPermission | SessionActivity::AwaitingUserInput => {
            ProgressState::Awaiting
        }
        SessionActivity::Blocked => ProgressState::Blocked,
    }
}

/// Computes the OSC 9;4 progress frames for the current state, or `None`
/// when nothing changed since the last emission.
pub(crate) fn progress_frames_if_changed(app: &App) -> Option<Vec<Vec<u8>>> {
    if !progress_operational(app) {
        return None;
    }
    let state = current_progress_state(app);
    if app.terminal_integration.last_progress() == Some(state) {
        return None;
    }
    Some(progress_frames(state))
}

/// Emits the OSC 9;4 progress frames for the current state, if changed.
pub(crate) fn sync_terminal_progress(
    app: &mut App,
    terminal: &mut TerminalRuntime,
) -> crate::Result<()> {
    let Some(frames) = progress_frames_if_changed(app) else {
        return Ok(());
    };
    let owned_frames = frames
        .iter()
        .map(|frame| frame.as_slice())
        .collect::<Vec<_>>();
    terminal.write_protocol_frames(&owned_frames)?;
    let state = current_progress_state(app);
    app.terminal_integration.note_progress_emitted(state);
    Ok(())
}

/// Emits at most one attention notification, if the terminal supports them.
/// Locally queued notifications (permission/user-input requests, flash
/// errors) take precedence; otherwise a one-shot lifecycle notification from
/// the `agena.terminal` plugin (run completed / blocked) is consumed.
/// Returns the emitted method (or `None` when nothing was queued or the
/// capability is disabled).
pub(crate) fn drain_terminal_notification(
    app: &mut App,
    terminal: &mut TerminalRuntime,
) -> crate::Result<Option<NotificationMethod>> {
    let local = app.terminal_integration.take_notification();
    let plugin_notify = local.is_none().then(|| take_plugin_notify(app)).flatten();
    let Some(method) = local.or(plugin_notify) else {
        return Ok(None);
    };
    if !notifications_operational(app) {
        return Ok(None);
    }
    let frames = notification_frames(method, &notification_summary(app));
    let owned_frames = frames
        .iter()
        .map(|frame| frame.as_slice())
        .collect::<Vec<_>>();
    terminal.write_protocol_frames(&owned_frames)?;
    Ok(Some(method))
}

/// Maps a plugin notification emitted through the unified `host.notify` entry
/// to a terminal notification method. Each distinct intent is recorded as
/// consumed once so a bell fires a single time even while the host keeps the
/// bounded recent queue around.
fn take_plugin_notify(app: &mut App) -> Option<NotificationMethod> {
    for notification in
        crate::app_backend::plugin_effects::plugin_host_notifications(&app.application)
    {
        let key = format!(
            "{}:{}:{}",
            notification.plugin_id, notification.severity, notification.body
        );
        if app.terminal_integration.notify_consumed_once(&key) {
            return current_notification_method(app);
        }
    }
    None
}

/// The localized notification summary text shown by the desktop notification
/// for the queued event. Derived from the current session and activity so the
/// alert is meaningful even when it lands after the user has tabbed away.
fn notification_summary(app: &App) -> String {
    let activity = app.current_session_activity();
    let session = app.current_or_selected_session_title().unwrap_or_default();
    let state = match activity {
        SessionActivity::Idle => None,
        SessionActivity::Running => Some(app.i18n.text("terminal-notification-working")),
        SessionActivity::AwaitingPermission => {
            Some(app.i18n.text("terminal-notification-permission"))
        }
        SessionActivity::AwaitingUserInput => {
            Some(app.i18n.text("terminal-notification-user-input"))
        }
        SessionActivity::Blocked => Some(app.i18n.text("terminal-notification-blocked")),
    };
    match (session, state) {
        (title, Some(state)) => format!("{title} · {state}"),
        (title, None) => title,
    }
}

/// Selects the notification method for the current terminal family, honoring
/// the capability gate. `None` when the terminal cannot raise attention.
pub(crate) fn current_notification_method(app: &App) -> Option<NotificationMethod> {
    if !notifications_operational(app) {
        return None;
    }
    let family = app
        .launch
        .terminal_context
        .as_ref()
        .map(|context| context.identity.family)
        .unwrap_or(agena_tui::terminal::TerminalFamily::Unknown);
    notification_method(family)
}
