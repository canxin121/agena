//! `agena.terminal` plugin: terminal window-title and notification state.
//!
//! This plugin is intentionally **hooks-only** — it exposes no tools and is
//! never visible to the AI. It observes the session lifecycle (run start/stop,
//! session start/end, agent stop, user-prompt submission) and publishes a
//! compact structured state that the TUI consumes to render the terminal
//! window title and to raise native attention notifications.
//!
//! The terminal *capability* detection (which OSC protocols the endpoint
//! supports) lives in the TUI detection layer, not here. This plugin only
//! decides *what state to show*; the TUI decides *how to render it*.

use std::sync::{Arc, RwLock};

use agena_plugin_host::sdk::host_api::{
    HostClient, HostStatuslineContributeRequest, HostStatuslineRemoveRequest,
};
use agena_plugin_host::sdk::{
    AgentStopInput, AgentStopPatch, InitContext, InitOutcome, PostRunInput, PreRunInput,
    Result as SdkResult, SessionEndInput, SessionStartInput, SessionStartPatch,
    UserPromptSubmitInput, UserPromptSubmitPatch,
};
use agena_plugin_host::PluginError;

pub(crate) const TERMINAL_PLUGIN_ID: &str = "agena.terminal";

/// Reserved statusline segment ids published by this plugin. The TUI reads
/// exactly these ids to reconstruct the terminal state.
pub(crate) const TITLE_SEGMENT_ID: &str = "agena.terminal.title";
pub(crate) const ACTIVITY_SEGMENT_ID: &str = "agena.terminal.activity";
/// One-shot attention notification intent. Written before `activity` so the
/// TUI can consume it on the next frame and clear it.
pub(crate) const NOTIFY_SEGMENT_ID: &str = "agena.terminal.notify";

/// A lifecycle event that should raise terminal attention.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TerminalNotify {
    /// A run completed successfully.
    Done,
    /// A run ended with an error / blocked state.
    Blocked,
}

/// The activity phase projected to the terminal title.
///
/// The plugin observes the session lifecycle hooks, so it can only distinguish
/// idle/running/blocked. Permission and user-input waits are not observable
/// through a plugin hook today; the TUI overlays those states locally when a
/// pending interactive request is present.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TerminalActivity {
    Idle,
    Running,
    Blocked,
}

pub(crate) struct TerminalPlugin {
    host: RwLock<Option<Arc<dyn HostClient>>>,
    activity: Arc<RwLock<TerminalActivity>>,
    notify: Arc<RwLock<Option<TerminalNotify>>>,
}

#[agena_plugin_host::sdk::agena_plugin(
    namespace = "agena",
    name = "terminal",
    version = env!("CARGO_PKG_VERSION"),
    summary = "Terminal window title and attention notification integration.",
    display = brief
)]
impl TerminalPlugin {
    pub(crate) fn new() -> Self {
        Self {
            host: RwLock::new(None),
            activity: Arc::new(RwLock::new(TerminalActivity::Idle)),
            notify: Arc::new(RwLock::new(None)),
        }
    }

    fn host(&self) -> SdkResult<Arc<dyn HostClient>> {
        self.host
            .read()
            .map_err(|_| PluginError::internal("terminal plugin host lock poisoned"))?
            .clone()
            .ok_or_else(|| PluginError::internal("terminal plugin invoked before init"))
    }

    /// Record the current activity (and optionally a one-shot notify intent)
    /// and publish it to the statusline channel so the TUI can render the
    /// terminal title and raise attention. Best-effort: a missing host or a
    /// failed write is non-fatal because the TUI retains a local fallback.
    async fn set_and_publish(&self, activity: TerminalActivity, notify: Option<TerminalNotify>) {
        if let Ok(mut guard) = self.activity.write() {
            *guard = activity;
        }
        if notify.is_some()
            && let Ok(mut guard) = self.notify.write()
        {
            *guard = notify;
        }
        self.publish_segments().await;
    }

    async fn publish_segments(&self) {
        let Ok(host) = self.host() else {
            return;
        };
        let activity = serde_json::to_value(
            self.activity
                .read()
                .map(|guard| *guard)
                .unwrap_or(TerminalActivity::Idle),
        )
        .unwrap_or_default()
        .to_string();
        let notify = self
            .notify
            .read()
            .ok()
            .and_then(|guard| *guard)
            .and_then(|kind| serde_json::to_value(kind).ok())
            .map(|value| value.to_string());
        // Publish notify before activity so the TUI can consume and clear it.
        if let Some(notify) = notify {
            let _ = host
                .ui_statusline_contribute(HostStatuslineContributeRequest {
                    segment_id: NOTIFY_SEGMENT_ID.to_owned(),
                    content: notify,
                    priority: i32::MAX + 1,
                    color: None,
                })
                .await;
        }
        let _ = host
            .ui_statusline_contribute(HostStatuslineContributeRequest {
                segment_id: ACTIVITY_SEGMENT_ID.to_owned(),
                content: activity,
                priority: i32::MAX,
                color: None,
            })
            .await;
        // The TUI consumes the notify intent by reading the segment. Keep the
        // segment present so it is not re-armed; the App clears it after firing
        // a notification.
    }

    /// Clear the statusline segments on shutdown.
    async fn clear_segments(&self) {
        let Ok(host) = self.host() else {
            return;
        };
        let _ = host
            .ui_statusline_remove(HostStatuslineRemoveRequest {
                segment_id: ACTIVITY_SEGMENT_ID.to_owned(),
            })
            .await;
        let _ = host
            .ui_statusline_remove(HostStatuslineRemoveRequest {
                segment_id: TITLE_SEGMENT_ID.to_owned(),
            })
            .await;
        let _ = host
            .ui_statusline_remove(HostStatuslineRemoveRequest {
                segment_id: NOTIFY_SEGMENT_ID.to_owned(),
            })
            .await;
    }

    #[hook(init)]
    async fn init(&self, _ctx: InitContext, host: Arc<dyn HostClient>) -> SdkResult<InitOutcome> {
        *self
            .host
            .write()
            .map_err(|_| PluginError::internal("terminal plugin host lock poisoned"))? = Some(host);
        Ok(InitOutcome::ack(agena_plugin_host::sdk::Plugin::manifest(
            self,
        )))
    }

    #[hook(shutdown)]
    async fn shutdown(&self) -> SdkResult<()> {
        self.clear_segments().await;
        Ok(())
    }

    #[hook(run.pre)]
    async fn pre_run(&self, _input: PreRunInput) -> SdkResult<()> {
        self.set_and_publish(TerminalActivity::Running, None).await;
        Ok(())
    }

    #[hook(run.post)]
    async fn post_run(&self, input: PostRunInput) -> SdkResult<()> {
        if input.status.trim().starts_with("error") {
            self.set_and_publish(TerminalActivity::Blocked, Some(TerminalNotify::Blocked))
                .await;
        } else {
            self.set_and_publish(TerminalActivity::Idle, Some(TerminalNotify::Done)).await;
        }
        Ok(())
    }

    #[hook(session.start)]
    async fn session_start(&self, _input: SessionStartInput) -> SdkResult<Option<SessionStartPatch>> {
        self.set_and_publish(TerminalActivity::Idle, None).await;
        Ok(None)
    }

    #[hook(session.end)]
    async fn session_end(&self, _input: SessionEndInput) -> SdkResult<()> {
        self.set_and_publish(TerminalActivity::Idle, None).await;
        Ok(())
    }

    #[hook(prompt.submit)]
    async fn user_prompt_submit(
        &self,
        _input: UserPromptSubmitInput,
    ) -> SdkResult<Option<UserPromptSubmitPatch>> {
        // The user submitted a prompt; the run will follow shortly. Mark the
        // state as running so the title reflects it before pre_run fires.
        self.set_and_publish(TerminalActivity::Running, None).await;
        Ok(None)
    }

    #[hook(agent.stop)]
    async fn agent_stop(&self, _input: AgentStopInput) -> SdkResult<Option<AgentStopPatch>> {
        self.set_and_publish(TerminalActivity::Idle, Some(TerminalNotify::Done)).await;
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agena_plugin_host::sdk::Plugin;

    #[test]
    fn manifest_exposes_no_tools() {
        let manifest = TerminalPlugin::new().manifest();
        assert_eq!(manifest.namespace, "agena");
        assert_eq!(manifest.name, "terminal");
        assert!(manifest.tools.is_empty(), "terminal plugin must expose no tools");
    }

    #[test]
    fn activity_serializes_to_snake_case_wire_format() {
        assert_eq!(
            serde_json::to_value(TerminalActivity::Running).unwrap(),
            serde_json::json!("running")
        );
        assert_eq!(
            serde_json::to_value(TerminalActivity::Blocked).unwrap(),
            serde_json::json!("blocked")
        );
    }
}
