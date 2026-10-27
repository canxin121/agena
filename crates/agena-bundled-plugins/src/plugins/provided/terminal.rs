//! `agena.terminal`: shared attention policy and terminal activity display.
//!
//! Canonical kernel execution and interaction facts produce persisted
//! `host.notify` intents, shared by the terminal and browser delivery adapters.
//! The observer lives with the plugin, independently of connected pages, and
//! stops when the plugin shuts down. Lifecycle hooks publish title activity;
//! they do not decide whether an AI execution has finished.
//!
//! The plugin exposes no AI tools. Terminal OSC capability detection and
//! browser Web Push delivery remain responsibilities of the respective adapters.

use std::sync::{Arc, Mutex, RwLock};

mod attention;
pub use attention::NotificationSource;

use agena_plugin_host::PluginError;
use agena_plugin_host::sdk::host_api::{
    HostClient, HostDisplayContributeRequest, HostDisplayRemoveRequest,
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
    notification_source: Option<NotificationSource>,
    notification_task: Mutex<Option<tokio::task::JoinHandle<()>>>,
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
    summary = "Shared terminal/browser attention notifications and terminal window titles.",
    translations(
        locale("zh-CN", summary = "统一终端和浏览器的提醒通知，并集成终端窗口标题。"),
        locale("zh-TW", summary = "統一終端機和瀏覽器的提醒通知，並整合終端機視窗標題。"),
        locale("ja-JP", summary = "ターミナルのウィンドウタイトル設定と注意喚起通知を連携します。"),
        locale("ko-KR", summary = "터미널 창 제목 설정과 주의 알림을 연동합니다."),
        locale("fr-FR", summary = "Intégration du titre de la fenêtre du terminal et des notifications d’attention."),
        locale("de-DE", summary = "Integration von Terminalfenstertiteln und Aufmerksamkeitshinweisen."),
        locale("es-ES", summary = "Integra el título de la ventana del terminal y las notificaciones de atención."),
        locale("hi-IN", summary = "टर्मिनल विंडो का शीर्षक और ध्यान-सूचनाएँ एकीकृत करता है।"),
        locale("ar-SA", summary = "تكامل عنوان نافذة الطرفية مع إشعارات التنبيه."),
        locale("pt-BR", summary = "Integra o título da janela do terminal e notificações de atenção.")
    ),
)]
impl TerminalPlugin {
    pub(crate) fn new() -> Self {
        Self {
            host: RwLock::new(None),
            activity: Arc::new(RwLock::new(TerminalActivity::Idle)),
            notification_source: None,
            notification_task: Mutex::new(None),
        }
    }

    pub(crate) fn with_notifications(mut self, source: Option<NotificationSource>) -> Self {
        self.notification_source = source;
        self
    }

    fn stop_notifications(&self) {
        if let Some(task) = self
            .notification_task
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
        {
            task.abort();
        }
    }

    fn host(&self) -> SdkResult<Arc<dyn HostClient>> {
        recover_read(&self.host, "read terminal plugin host")
            .clone()
            .ok_or_else(|| PluginError::internal("terminal plugin invoked before init"))
    }

    /// Record the current activity
    /// and publish it through the declarative display channel so the TUI can
    /// render the terminal title. Best-effort: a missing host or a failed
    /// write is non-fatal because the TUI retains a local fallback.
    async fn set_and_publish(&self, activity: TerminalActivity) {
        *recover_write(&self.activity, "update terminal activity") = activity;
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
        *recover_write(&self.host, "initialize terminal plugin host") = Some(host.clone());
        if let Some(source) = self.notification_source.clone() {
            let task = attention::start(source, host)
                .await
                .map_err(|error| PluginError::internal(error.to_string()))?;
            *self
                .notification_task
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(task);
        }
        Ok(InitOutcome::ack(agena_plugin_host::sdk::Plugin::manifest(
            self,
        )))
    }

    #[hook(shutdown)]
    async fn shutdown(&self) -> SdkResult<()> {
        self.stop_notifications();
        self.clear_display().await
    }

    #[hook(run.pre)]
    async fn pre_run(&self, _input: PreRunInput) -> SdkResult<()> {
        self.set_and_publish(TerminalActivity::Running).await;
        Ok(())
    }

    #[hook(run.post)]
    async fn post_run(&self, input: PostRunInput) -> SdkResult<()> {
        if input.status.trim().starts_with("error") {
            self.set_and_publish(TerminalActivity::Blocked).await;
        } else {
            // run.post is a provider round boundary, not agent completion.
            // Attention is emitted from committed kernel facts, not hooks.
            self.set_and_publish(TerminalActivity::Idle).await;
        }
        Ok(())
    }

    #[hook(session.start)]
    async fn session_start(
        &self,
        _input: SessionStartInput,
    ) -> SdkResult<Option<SessionStartPatch>> {
        self.set_and_publish(TerminalActivity::Idle).await;
        Ok(None)
    }

    #[hook(session.end)]
    async fn session_end(&self, _input: SessionEndInput) -> SdkResult<()> {
        self.set_and_publish(TerminalActivity::Idle).await;
        Ok(())
    }

    #[hook(prompt.submit)]
    async fn user_prompt_submit(
        &self,
        _input: UserPromptSubmitInput,
    ) -> SdkResult<Option<UserPromptSubmitPatch>> {
        // The user submitted a prompt; the run will follow shortly. Mark the
        // state as running so the title reflects it before pre_run fires.
        self.set_and_publish(TerminalActivity::Running).await;
        Ok(None)
    }

    #[hook(agent.stop)]
    async fn agent_stop(&self, input: AgentStopInput) -> SdkResult<Option<AgentStopPatch>> {
        if input.run_error.is_some() {
            self.set_and_publish(TerminalActivity::Blocked).await;
        } else {
            self.set_and_publish(TerminalActivity::Idle).await;
        }
        Ok(None)
    }
}

impl Drop for TerminalPlugin {
    fn drop(&mut self) {
        if let Some(task) = self
            .notification_task
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
        {
            task.abort();
        }
    }
}
