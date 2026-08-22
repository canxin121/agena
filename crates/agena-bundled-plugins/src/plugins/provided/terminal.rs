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

use agena_plugin_host::PluginError;
use agena_plugin_host::sdk::host_api::{
    HostClient, HostDisplayContributeRequest, HostDisplayRemoveRequest, PluginNotifyRequest,
};
use agena_plugin_host::sdk::{
    AgentStopInput, AgentStopPatch, ContributionKind, InitContext, InitOutcome,
    PluginDisplayContent, PluginDisplayContribution, PostRunInput, PreRunInput,
    Result as SdkResult, SessionEndInput, SessionStartInput, SessionStartPatch,
    UserPromptSubmitInput, UserPromptSubmitPatch,
};

pub(crate) const TERMINAL_PLUGIN_ID: &str = "agena.terminal";

/// Display contribution id published by this plugin. The TUI reads exactly
/// this id to reconstruct the terminal activity state.
pub(crate) const ACTIVITY_CONTRIBUTION_ID: &str = "agena.terminal.activity";

/// Display priority for the activity contribution. One-shot attention uses
/// the unified `host.notify` entry; the activity state rides the declarative
/// display channel (`host.display_contribute`).
pub(crate) const ACTIVITY_CONTRIBUTION_PRIORITY: i32 = i32::MAX - 1;

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

/// Build the unified notify request for a terminal lifecycle event (Phase 6).
/// The host decides surface/priority; the TUI consumes it for terminal bells.
fn notify_request(kind: TerminalNotify) -> PluginNotifyRequest {
    PluginNotifyRequest {
        title: String::new(),
        body: match kind {
            TerminalNotify::Done => "run completed".to_owned(),
            TerminalNotify::Blocked => "run blocked".to_owned(),
        },
        severity: match kind {
            TerminalNotify::Done => "success".to_owned(),
            TerminalNotify::Blocked => "error".to_owned(),
        },
        session_id: None,
        actions: Vec::new(),
    }
}

fn recover_read<'a, T>(lock: &'a RwLock<T>, context: &str) -> std::sync::RwLockReadGuard<'a, T> {
    match lock.read() {
        Ok(guard) => guard,
        Err(error) => {
            tracing::error!(
                operation = context,
                error = %error,
                "recovering poisoned terminal plugin read lock"
            );
            error.into_inner()
        }
    }
}

fn recover_write<'a, T>(lock: &'a RwLock<T>, context: &str) -> std::sync::RwLockWriteGuard<'a, T> {
    match lock.write() {
        Ok(guard) => guard,
        Err(error) => {
            tracing::error!(
                operation = context,
                error = %error,
                "recovering poisoned terminal plugin write lock"
            );
            error.into_inner()
        }
    }
}

#[agena_plugin_host::sdk::agena_plugin(
    namespace = "agena",
    name = "terminal",
    version = env!("CARGO_PKG_VERSION"),
    summary = "Terminal window title and attention notification integration.",
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
        recover_read(&self.host, "read terminal plugin host")
            .clone()
            .ok_or_else(|| PluginError::internal("terminal plugin invoked before init"))
    }

    /// Record the current activity (and optionally a one-shot notify intent)
    /// and publish it through the declarative display channel so the TUI can
    /// render the terminal title. Best-effort: a missing host or a failed
    /// write is non-fatal because the TUI retains a local fallback.
    async fn set_and_publish(&self, activity: TerminalActivity, notify: Option<TerminalNotify>) {
        *recover_write(&self.activity, "update terminal activity") = activity;
        if notify.is_some() {
            *recover_write(&self.notify, "update terminal notification intent") = notify;
        }
        self.publish_display().await;
    }

    async fn publish_display(&self) {
        let host = match self.host() {
            Ok(host) => host,
            Err(error) => {
                tracing::warn!(
                    diagnostic = %error.diagnostic.message,
                    "terminal display publication skipped because the host is unavailable"
                );
                return;
            }
        };
        // One-shot attention notification via the unified notify entry. The
        // host decides surface; the TUI consumes it for terminal bells.
        let notify = *recover_read(&self.notify, "read terminal notification intent");
        if let Some(kind) = notify
            && let Err(error) = host.notify(notify_request(kind)).await
        {
            tracing::warn!(
                diagnostic = %error.diagnostic.message,
                "failed to publish terminal attention notification"
            );
        }
        let activity = *recover_read(&self.activity, "read terminal activity");
        let value = match activity {
            TerminalActivity::Idle => "idle",
            TerminalActivity::Running => "running",
            TerminalActivity::Blocked => "blocked",
        };
        if let Err(error) = host
            .display_contribute(HostDisplayContributeRequest {
                contribution: PluginDisplayContribution {
                    id: ACTIVITY_CONTRIBUTION_ID.to_owned(),
                    kind: ContributionKind::TerminalActivity,
                    priority: ACTIVITY_CONTRIBUTION_PRIORITY,
                    content: PluginDisplayContent::TerminalActivity {
                        value: value.to_owned(),
                    },
                },
            })
            .await
        {
            tracing::warn!(
                diagnostic = %error.diagnostic.message,
                "failed to publish terminal activity display"
            );
        }
    }

    /// Clear the display contribution on shutdown.
    async fn clear_display(&self) -> SdkResult<()> {
        let host = self.host()?;
        host.display_remove(HostDisplayRemoveRequest {
            contribution_id: ACTIVITY_CONTRIBUTION_ID.to_owned(),
        })
        .await
        .map(|_| ())
    }

    #[hook(init)]
    async fn init(&self, _ctx: InitContext, host: Arc<dyn HostClient>) -> SdkResult<InitOutcome> {
        *recover_write(&self.host, "initialize terminal plugin host") = Some(host);
        Ok(InitOutcome::ack(agena_plugin_host::sdk::Plugin::manifest(
            self,
        )))
    }

    #[hook(shutdown)]
    async fn shutdown(&self) -> SdkResult<()> {
        self.clear_display().await
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
            self.set_and_publish(TerminalActivity::Idle, Some(TerminalNotify::Done))
                .await;
        }
        Ok(())
    }

    #[hook(session.start)]
    async fn session_start(
        &self,
        _input: SessionStartInput,
    ) -> SdkResult<Option<SessionStartPatch>> {
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
        self.set_and_publish(TerminalActivity::Idle, Some(TerminalNotify::Done))
            .await;
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
        assert!(
            manifest.tools.is_empty(),
            "terminal plugin must expose no tools"
        );
    }

    #[test]
    fn activity_contribution_priority_stays_high() {
        assert_eq!(ACTIVITY_CONTRIBUTION_PRIORITY, i32::MAX - 1);
    }

    #[test]
    fn notify_request_maps_terminal_lifecycle_to_severity() {
        let done = notify_request(TerminalNotify::Done);
        assert_eq!(done.body, "run completed");
        assert_eq!(done.severity, "success");
        assert!(done.actions.is_empty());

        let blocked = notify_request(TerminalNotify::Blocked);
        assert_eq!(blocked.body, "run blocked");
        assert_eq!(blocked.severity, "error");
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
