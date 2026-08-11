use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    future::Future,
    path::PathBuf,
    pin::Pin,
    sync::{Arc, OnceLock},
    time::{Duration, Instant},
};

use agena_api::{
    pagination::PaginatedResponse,
    resource::{
        ProviderAdapterModelsResponse, ProviderSummaryResource, SessionExecutionResource,
        SessionResource,
    },
};
use agena_domain::ModelRef;
use agena_domain::PermissionMode;
use agena_domain::PermissionReplyKind;
use agena_domain::UsagePeriod;
use agena_domain::UsageStats;
use tokio::sync::mpsc::{Receiver, Sender};

use crate::app_backend::{LiveEvent, SessionPermissionStudioState, SessionRefresh};
use crate::composer_queue::ComposerQueue;
use crate::notifications::NotificationStore;
use agena_application::dto::ModelCatalogListResponse;
use agena_tui::composer::ComposerItemSelection;
use agena_tui::i18n::I18n;
use agena_tui::input::ComposerKeyBindings;
use agena_tui::main_focus::Focus;
use agena_tui::presentation_config::TuiConfig;
use agena_tui::status_line::StatusLinePresentation;
use agena_tui::usage::{UsageDashboardData, UsageDashboardPresentation};
use agena_tui_components::{Editor, InputDialogState};
use agena_tui_media::{MathGraphicsConfig, MathGraphicsRenderer, MathRenderContext};
use agena_tui_platform::terminal::TerminalContext;
use agena_tui_session::session_list::SessionListPresentation;
use agena_tui_session::session_view::SessionViewMode;
use agena_tui_settings::SettingsStudioSectionId;

use super::{PluginWorkbenchOverlay, cleanup_temporary_composer_items};

mod composer;
pub(crate) use self::composer::*;
mod overlays;
pub(crate) use self::overlays::*;
mod session;
pub(crate) use self::session::*;
pub(crate) use crate::transcript_state::TranscriptViewportRow;
pub(crate) use agena_tui_transcript::{
    LayoutCache, RenderedLine, RenderedTranscript, RenderedTranscriptNode, TranscriptAction,
    TranscriptBlockCursor, TranscriptBlockSelectionMode, TranscriptClick, TranscriptContentId,
    TranscriptCursor, TranscriptCursorAnchor, TranscriptDetailDefaults, TranscriptInteraction,
    TranscriptMoveDirection, TranscriptNodeKey, TranscriptNodeKind, TranscriptPointerGesture,
    TranscriptScrollbarDrag, TranscriptTextPosition, TranscriptTextSelection, TranscriptViewport,
    TranscriptVisualSelectionMode, TranscriptVisualSelectionSnapshot,
};

#[cfg(test)]
pub(crate) use agena_tui_transcript::TranscriptVerticalNavigationStep;

pub(super) const TIMELINE_EVENT_LIMIT: u64 = 200;
// A full ratatui frame can include Markdown layout, syntax highlighting and
// rich tool cards. Ten frames per second keeps spinners responsive without
// continuously re-rendering a large transcript at ~31 FPS while a tool waits.
pub(super) const UI_TICK_MS: u64 = 100;
pub(super) const UI_COMMAND_QUEUE_CAPACITY: usize = 64;
pub(super) const UI_COMMAND_MAX_CONCURRENCY: usize = 8;
pub(super) const APP_MESSAGE_QUEUE_CAPACITY: usize = 512;
pub(super) const REFRESH_INTERVAL_MS: u64 = 250;
/// Maximum time an in-flight session refresh or state load may remain
/// unresolved before the TUI force-clears the in-flight flag and resumes
/// periodic refresh. A healthy refresh completes far faster (the SQLite busy
/// timeout is 15s); a lost response (spawned task panicked or its message was
/// dropped) would otherwise wedge the flag forever and freeze the transcript
/// at a stale snapshot with stuck \"working\" indicators.
pub(super) const REFRESH_STALL_TIMEOUT_MS: u64 = 30_000;
pub(super) const DRAFT_PERSIST_INTERVAL_MS: u64 = 250;
/// Retry cadence for healing the composer's bottom-right plan chip after a
/// runtime reload or process restart, when the in-memory display contribution
/// starts empty. Only fires while a session is attached and the chip is
/// missing; sessions known to have no plan back off much longer.
pub(super) const PLAN_DISPLAY_REFRESH_RETRY_MS: u64 = 2_000;
/// Backoff used when the last plan read reported no active plan, so sessions
/// without a plan do not trigger a plan lookup on every retry window.
pub(super) const PLAN_DISPLAY_REFRESH_NO_PLAN_BACKOFF_MS: u64 = 30_000;
pub(super) const MAX_FILE_MENTION_SUGGESTIONS: usize = 100;
pub(super) const MAX_PROMPT_HISTORY_ENTRIES: usize = 200;
pub(super) const AWS_REGION_CHOICES: &[&str] = &[
    "us-east-1",
    "us-east-2",
    "us-west-1",
    "us-west-2",
    "ca-central-1",
    "sa-east-1",
    "eu-west-1",
    "eu-west-2",
    "eu-west-3",
    "eu-central-1",
    "eu-central-2",
    "eu-north-1",
    "eu-south-1",
    "eu-south-2",
    "ap-south-1",
    "ap-south-2",
    "ap-east-1",
    "ap-southeast-1",
    "ap-southeast-2",
    "ap-southeast-3",
    "ap-southeast-4",
    "ap-northeast-1",
    "ap-northeast-2",
    "ap-northeast-3",
    "me-south-1",
    "me-central-1",
    "af-south-1",
];

pub(super) fn settings_fields() -> Vec<SettingsFieldSpec> {
    vec![
        SettingsFieldSpec {
            section: SettingsStudioSectionId::ModelsProviders,
            path: "providers.default".to_string(),
            label_key: "settings-field-default-provider-label",
            description_key: "settings-field-default-provider-description",
            kind: SettingsFieldKind::String,
            label_override: None,
            description_override: None,
        },
        SettingsFieldSpec {
            section: SettingsStudioSectionId::Interface,
            path: "ui.locale".to_string(),
            label_key: "settings-field-ui-locale-label",
            description_key: "settings-field-ui-locale-description",
            kind: SettingsFieldKind::String,
            label_override: None,
            description_override: None,
        },
        SettingsFieldSpec {
            section: SettingsStudioSectionId::Interface,
            path: "ui.tui.color_scheme".to_string(),
            label_key: "settings-field-tui-color-scheme-label",
            description_key: "settings-field-tui-color-scheme-description",
            kind: SettingsFieldKind::String,
            label_override: None,
            description_override: None,
        },
        SettingsFieldSpec {
            section: SettingsStudioSectionId::Interface,
            path: "ui.tui.graphics".to_string(),
            label_key: "settings-field-tui-graphics-label",
            description_key: "settings-field-tui-graphics-description",
            kind: SettingsFieldKind::String,
            label_override: None,
            description_override: None,
        },
        SettingsFieldSpec {
            section: SettingsStudioSectionId::Interface,
            path: "ui.tui.theme".to_string(),
            label_key: "settings-field-tui-theme-label",
            description_key: "settings-field-tui-theme-description",
            kind: SettingsFieldKind::String,
            label_override: None,
            description_override: None,
        },
        SettingsFieldSpec {
            section: SettingsStudioSectionId::Interface,
            path: "ui.tui.transcript.activity_default_expanded".to_string(),
            label_key: "settings-field-activity-default-expanded-label",
            description_key: "settings-field-activity-default-expanded-description",
            kind: SettingsFieldKind::Bool,
            label_override: None,
            description_override: None,
        },
        SettingsFieldSpec {
            section: SettingsStudioSectionId::Diagnostics,
            path: "tracing.filter".to_string(),
            label_key: "settings-field-tracing-filter-label",
            description_key: "settings-field-tracing-filter-description",
            kind: SettingsFieldKind::String,
            label_override: None,
            description_override: None,
        },
        SettingsFieldSpec {
            section: SettingsStudioSectionId::Diagnostics,
            path: "tracing.database".to_string(),
            label_key: "settings-field-tracing-database-label",
            description_key: "settings-field-tracing-database-description",
            kind: SettingsFieldKind::String,
            label_override: None,
            description_override: None,
        },
        SettingsFieldSpec {
            section: SettingsStudioSectionId::Diagnostics,
            path: "tracing.adapter".to_string(),
            label_key: "settings-field-tracing-adapter-label",
            description_key: "settings-field-tracing-adapter-description",
            kind: SettingsFieldKind::String,
            label_override: None,
            description_override: None,
        },
        SettingsFieldSpec {
            section: SettingsStudioSectionId::ProviderClientVersions,
            path: "runtime.providers.client_versions.codex".to_string(),
            label_key: "settings-field-runtime-codex-version-label",
            description_key: "settings-field-runtime-codex-version-description",
            kind: SettingsFieldKind::String,
            label_override: None,
            description_override: None,
        },
        SettingsFieldSpec {
            section: SettingsStudioSectionId::ProviderClientVersions,
            path: "runtime.providers.client_versions.claude".to_string(),
            label_key: "settings-field-runtime-claude-version-label",
            description_key: "settings-field-runtime-claude-version-description",
            kind: SettingsFieldKind::String,
            label_override: None,
            description_override: None,
        },
        SettingsFieldSpec {
            section: SettingsStudioSectionId::ProviderClientVersions,
            path: "runtime.providers.client_versions.gemini".to_string(),
            label_key: "settings-field-runtime-gemini-version-label",
            description_key: "settings-field-runtime-gemini-version-description",
            kind: SettingsFieldKind::String,
            label_override: None,
            description_override: None,
        },
        SettingsFieldSpec {
            section: SettingsStudioSectionId::RuntimeSession,
            path: "session.compaction.auto".to_string(),
            label_key: "settings-field-session-compaction-auto-label",
            description_key: "settings-field-session-compaction-auto-description",
            kind: SettingsFieldKind::Bool,
            label_override: None,
            description_override: None,
        },
        SettingsFieldSpec {
            section: SettingsStudioSectionId::RuntimeSession,
            path: "session.compaction.reserved_tokens".to_string(),
            label_key: "settings-field-session-compaction-reserved-tokens-label",
            description_key: "settings-field-session-compaction-reserved-tokens-description",
            kind: SettingsFieldKind::Integer,
            label_override: None,
            description_override: None,
        },
    ]
}

#[derive(Debug, Clone, Default)]
/// Options for launching the TUI app.
pub struct LaunchOptions {
    pub initial_session_id: Option<i64>,
    pub initial_session_search: Option<String>,
    pub tui_config: TuiConfig,
    pub terminal_background: Option<agena_tui_components::TerminalRgb>,
    pub terminal_context: Option<TerminalContext>,
    pub math_graphics: Option<MathGraphicsConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SessionLoadScope {
    pub(super) mode: SessionViewMode,
    pub(super) anchor_session_id: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum DraftSlot {
    Session(i64),
    NewSession,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum RunActivityTarget {
    Session(i64),
    NewSession,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum RunOperation {
    CreateSession,
    SubmitMessage,
    Continue,
    Compact,
    Rewind,
    PermissionReply,
    UserInputReply,
}

#[derive(Debug, Default)]
pub(super) struct RunActivityTracker {
    requests: BTreeMap<(RunActivityTarget, RunOperation), usize>,
}

impl RunActivityTracker {
    pub(super) fn begin(&mut self, target: RunActivityTarget, operation: RunOperation) {
        *self.requests.entry((target, operation)).or_default() += 1;
    }

    pub(super) fn finish(&mut self, target: RunActivityTarget, operation: RunOperation) {
        let key = (target, operation);
        let Some(count) = self.requests.get_mut(&key) else {
            return;
        };
        *count = count.saturating_sub(1);
        if *count == 0 {
            self.requests.remove(&key);
        }
    }

    pub(super) fn clear_session(&mut self, session_id: i64) {
        self.requests.retain(
            |(target, _), _| !matches!(target, RunActivityTarget::Session(id) if *id == session_id),
        );
    }

    pub(super) fn is_active(&self, target: RunActivityTarget) -> bool {
        self.requests
            .keys()
            .any(|(candidate, _)| *candidate == target)
    }

    pub(super) fn has_operation(&self, target: RunActivityTarget, operation: RunOperation) -> bool {
        self.requests.contains_key(&(target, operation))
    }
}

#[derive(Debug, Clone, Default)]
pub(super) struct DraftStore {
    pub(super) drafts: BTreeMap<DraftSlot, ComposerDraft>,
}

#[derive(Debug, Clone, Default)]
pub(super) struct PromptHistory {
    pub(super) items: Vec<String>,
}

/// Terminal-integration presentation state owned by `App`.
///
/// The run loop compares the current session title and activity against this
/// snapshot every frame and emits title changes through
/// `TerminalRuntime::write_protocol`. Attention notifications are queued by
/// session event handlers and drained one-per-frame so a burst of events
/// produces a single alert.
#[derive(Debug, Default)]
pub(super) struct TerminalIntegrationState {
    pub(super) last_title: Option<String>,
    pub(super) pending_notifications:
        Vec<agena_tui_platform::terminal::integration::NotificationMethod>,
    title_due: bool,
    /// Last OSC 9;4 progress state emitted; `None` before the first emission
    /// or while the terminal does not support progress.
    last_progress: Option<agena_tui_platform::terminal::integration::ProgressState>,
    /// Content of the `agena.terminal.notify` segment already fired. The
    /// plugin keeps the segment present across frames; this set ensures each
    /// lifecycle intent fires exactly once.
    consumed_notify: std::collections::BTreeSet<String>,
}

impl TerminalIntegrationState {
    /// Queues an attention notification for the next frame.
    pub(super) fn queue_notification(
        &mut self,
        method: agena_tui_platform::terminal::integration::NotificationMethod,
    ) {
        self.pending_notifications.push(method);
    }

    /// Marks a title emission as due. Called whenever the session title or
    /// activity changes.
    pub(super) fn mark_title_pending(&mut self) {
        self.title_due = true;
    }

    /// Returns the last emitted title; `None` before the first emission.
    pub(super) fn last_title(&self) -> Option<&str> {
        self.last_title.as_deref()
    }

    /// Records that the current title text has been emitted.
    pub(super) fn note_title_emitted(&mut self, title: String) {
        self.last_title = Some(title);
        self.title_due = false;
    }

    pub(super) fn title_due(&self) -> bool {
        self.title_due
    }

    /// Drains the queued notifications, returning at most one method (the
    /// most recently queued). Bursts coalesce into a single alert.
    pub(super) fn take_notification(
        &mut self,
    ) -> Option<agena_tui_platform::terminal::integration::NotificationMethod> {
        self.pending_notifications.drain(..).next_back()
    }

    /// Returns `true` when `notify_content` (from the `agena.terminal.notify`
    /// display contribution) has not yet fired, and records it as consumed.
    pub(super) fn notify_consumed_once(&mut self, notify_content: &str) -> bool {
        if self.consumed_notify.contains(notify_content) {
            return false;
        }
        self.consumed_notify.insert(notify_content.to_owned());
        if self.consumed_notify.len() > 64 {
            self.consumed_notify.clear();
        }
        true
    }

    /// Returns the last emitted progress state.
    pub(super) fn last_progress(
        &self,
    ) -> Option<agena_tui_platform::terminal::integration::ProgressState> {
        self.last_progress
    }

    /// Records that `state` has been emitted to the terminal chrome.
    pub(super) fn note_progress_emitted(
        &mut self,
        state: agena_tui_platform::terminal::integration::ProgressState,
    ) {
        self.last_progress = Some(state);
    }
}

/// The TUI application.
/// Cooldown state for healing the composer's bottom-right plan chip after a
/// restart or runtime reload. `result` is `None` while a refresh is in flight
/// or after a failed lookup, `Some(true)` when the session has an active
/// plan, and `Some(false)` when the last read found none.
#[derive(Debug, Clone, Copy)]
pub(super) struct PlanDisplayRefreshState {
    pub(super) session_id: i64,
    pub(super) requested_at: Instant,
    pub(super) result: Option<bool>,
}

pub struct App {
    pub(super) application: agena_application::Application,
    /// Lazy workspace file index for file mentions. `Application` carries no
    /// file index; the TUI owns the cached directory listing.
    pub(super) file_index: Arc<OnceLock<Vec<PathBuf>>>,
    pub(super) i18n: I18n,
    pub(super) tx: Sender<AppMessage>,
    pub(super) rx: Receiver<AppMessage>,
    /// Bounded ingress for backend work initiated by synchronous key/mouse
    /// handlers. The actor owns concurrency and completion delivery so UI
    /// code never blocks a Tokio worker to obtain an async result.
    pub(super) command_tx: Sender<UiCommand>,
    pub(super) command_rx: Option<Receiver<UiCommand>>,
    pub(super) command_actor: Option<tokio::task::JoinHandle<()>>,
    pub(super) settings_runtime_snapshot_summary: Option<String>,
    pub(super) settings_session_permission: Option<(i64, SessionPermissionStudioState)>,
    pub(super) session_selection_revision: u64,
    pub(super) launch: LaunchOptions,
    pub(super) math_renderer: Option<MathGraphicsRenderer>,
    pub(super) math_render_context: MathRenderContext,
    pub(super) should_quit: bool,
    pub(super) focus: Focus,
    pub(super) current_route: Route,
    pub(super) route_stack: Vec<Route>,
    pub(super) overlay: Option<Overlay>,
    pub(super) overlay_stack: Vec<Overlay>,
    pub(super) context_help: Option<HelpOverlay>,
    pub(super) seen_permission_request_ids: BTreeSet<String>,
    pub(super) seen_user_input_request_ids: BTreeSet<String>,
    pub(super) notifications: NotificationStore,
    pub(super) seen_failure_ids: HashSet<agena_failure::FailureId>,
    pub(super) sessions: SessionListPresentation,
    pub(super) session_load: SessionListLoadState,
    pub(super) session_composer: SessionComposerState,
    pub(super) session_controller: agena_tui_session::SessionController,
    pub(super) transcript: TranscriptState,
    pub(super) run_options: RunOptionsState,
    pub(super) composer: Editor,
    pub(super) composer_items: Vec<ComposerItem>,
    pub(super) slash_command_suggestions: Option<SlashCommandSuggestionState>,
    pub(super) slash_command_suggestion_actions: BTreeMap<String, SlashCommandSuggestionAction>,
    pub(super) dismissed_slash_command_suggestions_for: Option<String>,
    pub(super) file_mention_suggestions: Option<FileMentionSuggestionState>,
    pub(super) file_mention_suggestion_actions: BTreeMap<String, FileMentionSuggestionAction>,
    pub(super) dismissed_file_mention_suggestions_for: Option<String>,
    pub(super) prompt_history_search: Option<PromptHistorySearchState>,
    pub(super) composer_item_selection: ComposerItemSelection,
    pub(super) draft_store: DraftStore,
    pub(super) draft_store_path: PathBuf,
    pub(super) draft_store_dirty: bool,
    pub(super) draft_store_last_persist_at: Instant,
    pub(super) draft_store_reported_error: Option<String>,
    pub(super) pending_draft_store_error: Option<UiFailure>,
    pub(super) prompt_history: PromptHistory,
    pub(super) prompt_history_path: PathBuf,
    pub(super) prompt_history_reported_error: Option<String>,
    pub(super) pending_prompt_history_error: Option<UiFailure>,
    pub(super) run_activity: RunActivityTracker,
    pub(super) next_pending_user_message_id: u64,
    pub(super) layout: LayoutCache,
    pub(super) surface_layout: crate::SurfaceLayout,
    pub(super) surface_selection: Option<crate::SurfaceSelection>,
    pub(super) transcript_scrollbar_drag: Option<TranscriptScrollbarDrag>,
    pub(super) transcript_pointer_gesture: Option<TranscriptPointerGesture>,
    pub(super) last_transcript_click: Option<TranscriptClick>,
    pub(super) mouse_events_seen: u64,
    pub(super) last_mouse_event: Option<String>,
    pub(super) bootstrap_done: bool,
    pub(super) last_refresh_at: Instant,
    /// A refresh request that arrived while another refresh was already in
    /// flight. `request_refresh` used to drop it, which could stall the
    /// transcript until the next event or a restart; the refreshed handler
    /// re-issues it once the in-flight request completes.
    pub(super) pending_refresh: Option<(i64, bool)>,
    /// An event-driven refresh request merged into the periodic tick budget.
    /// Streaming flushes emit `PartUpdated` at far higher rates than the TUI
    /// can apply full-snapshot refreshes (~100+/s while reasoning); refreshing
    /// on every event leaves the backlog permanently behind the run, so the
    /// transcript only converges when the run ends. `on_tick` consumes this
    /// flag under the same `REFRESH_INTERVAL_MS` gate, repainting streamed
    /// parts at most once per 250 ms.
    pub(super) refresh_pending: bool,
    pub(super) pending_ui_action: Option<UiAction>,
    pub(super) current_lineage: Option<CurrentLineageState>,
    /// Open side conversations: side session id -> the parent session it was
    /// forked from. A side conversation is `/fork` plus a transient side
    /// identity: the fork itself is a permanent child session with full
    /// history, tracked here only while the TUI treats it as an open side
    /// conversation (`/side`; the user is switched into it to
    /// chat while the parent run keeps going). The entry is dropped when
    /// navigation leaves the side conversation or its parent.
    pub(super) side_sessions: HashMap<i64, i64>,
    /// Monotonic id for usage dashboard loads. Keeping it on the app prevents
    /// a late response from an older, closed dashboard matching a newly
    /// opened dashboard that is also on its first request.
    pub(super) next_usage_request_id: u64,
    /// Forwarder task that pumps `app_backend::live_events::subscribe_session_events` into
    /// [`AppMessage::SessionEventArrived`]. Aborted whenever the active
    /// session changes so we don't accumulate stale subscriptions.
    pub(super) active_subscription: Option<tokio::task::JoinHandle<()>>,
    /// Pending messages typed by the user while the AI was working. Drained
    /// Single pending message delivered once the active run finishes. See
    /// `composer_queue.rs`.
    pub(super) queue: ComposerQueue,
    pub(super) status_line: Option<StatusLinePresentation>,
    pub(super) plugin_theme: Option<agena_plugin_host::HostThemePalette>,
    pub(super) keybindings: ComposerKeyBindings,
    pub(super) transcript_motion_prefix: Option<String>,
    pub(super) transcript_yank_pending: bool,
    pub(super) transcript_yank_origin: Option<TranscriptTextPosition>,
    pub(super) transcript_goto_pending: bool,
    pub(super) transcript_viewport_pending: bool,
    /// `(forward, till, count)` while waiting for the character in an
    /// `f`/`F`/`t`/`T` command.
    pub(super) transcript_find_pending: Option<(bool, bool, usize)>,
    /// Most recently completed `f`/`F`/`t`/`T` request, used by `;` and `,`.
    pub(super) transcript_last_find: Option<(bool, bool, char)>,
    /// `(yank, around)` after `a` or `i` starts a Transcript text object.
    /// `yank` distinguishes `yam`/`yaM` from `vam`/`vaM`; `around` keeps the
    /// normal Vim distinction for built-in text objects such as `aw`/`iw`.
    pub(super) transcript_text_object_pending: Option<(bool, bool)>,
    /// Direction selected when the transcript search overlay was opened.
    pub(super) transcript_search_forward: bool,
    /// Last time the user pressed Ctrl+C; a second press within the window
    /// exits the application.
    pub(super) last_ctrl_c_at: Option<Instant>,
    pub(super) double_esc_window: Duration,
    pub(super) terminal_integration: TerminalIntegrationState,
    /// Most recent active background-activity count for the footer pill,
    /// refreshed on a slow cadence while the main route is visible.
    pub(super) background_activity_summary: Option<(usize, Instant)>,
    /// Cooldown state for healing the composer's bottom-right plan chip.
    /// The chip is backed by an in-memory display contribution that starts
    /// empty after a process restart or runtime reload; the TUI re-requests
    /// it (read-only `agena.plan.get`) on session open and periodically while
    /// the chip is missing.
    pub(super) plan_display_refresh: Option<PlanDisplayRefreshState>,
}

impl Drop for App {
    fn drop(&mut self) {
        self.rx.close();
        if let Some(actor) = self.command_actor.take() {
            actor.abort();
        }
        if let Some(subscription) = self.active_subscription.take() {
            subscription.abort();
        }
        self.sync_current_draft_slot();
        let _ = self.try_persist_draft_store(true);
        cleanup_temporary_composer_items(self.composer_items.as_slice());
        self.cleanup_temporary_draft_store_items();
    }
}

pub(super) enum AppMessage {
    AsyncOperationCompleted(UiCompletion),
    BackgroundActivitySummaryLoaded {
        count: usize,
    },
    ActivitiesLoaded {
        request_id: u64,
        result: UiResult<Vec<agena_tui::activities::ActivitiesRow>>,
    },
    ActivitiesLogLoaded {
        activity_id: String,
        request_id: u64,
        result: UiResult<agena_tui::activities::ActivitiesLogTail>,
    },
    ActivitiesStopped {
        activity_id: String,
        request_id: u64,
        result: UiResult<bool>,
    },
    ActivitiesDismissed {
        activity_id: String,
        request_id: u64,
        result: UiResult<bool>,
    },
    ActivitiesCleared {
        request_id: u64,
        result: UiResult<bool>,
    },
    PlanViewerLoaded {
        request_id: u64,
        result: UiResult<PlanViewerData>,
    },
    PlanAutorunToggled {
        request_id: u64,
        result: UiResult<bool>,
    },
    PlanDisplayRefreshed {
        session_id: i64,
        result: UiResult<bool>,
    },
    UsageStatsLoaded {
        request_id: u64,
        result: UiResult<UsageStats>,
    },
    SessionsLoaded {
        scope: SessionLoadScope,
        subtree_root_id: Option<i64>,
        result: UiResult<Vec<SessionResource>>,
    },
    SessionCreated {
        submit_draft: Option<ComposerDraft>,
        pending_message_id: Option<u64>,
        result: UiResult<SessionResource>,
    },
    SessionStateLoaded {
        session_id: i64,
        result: UiResult<SessionExecutionResource>,
    },
    SessionRefreshed {
        session_id: i64,
        result: UiResult<SessionRefresh>,
    },
    SessionMessageSubmitted {
        session_id: i64,
        pending_message_id: u64,
        draft: ComposerDraft,
        result: UiResult<SessionExecutionResource>,
    },
    SessionContinued {
        session_id: i64,
        result: UiResult<SessionExecutionResource>,
    },
    SessionCompacted {
        session_id: i64,
        result: UiResult<SessionExecutionResource>,
    },
    SessionRenamed {
        session_id: i64,
        result: UiResult<SessionResource>,
    },
    PermissionReplied {
        session_id: i64,
        request_id: String,
        kind: PermissionReplyKind,
        label: String,
        result: UiResult<SessionExecutionResource>,
    },
    UserInputReplied {
        session_id: i64,
        request_id: String,
        result: UiResult<SessionExecutionResource>,
    },
    ProvidersLoaded {
        purpose: ProviderPickerPurpose,
        result: UiResult<Vec<ProviderSummaryResource>>,
    },
    SessionSearchPageLoaded {
        mode: SessionViewMode,
        query: String,
        page_index: usize,
        result: UiResult<PaginatedResponse<SessionResource>>,
    },
    SessionSearchSubtreeLoaded {
        session_id: i64,
        query: String,
        result: UiResult<Vec<SessionResource>>,
    },
    LineageLoaded {
        session_id: i64,
        result: UiResult<Vec<SessionResource>>,
    },
    RewindMessagesLoaded {
        session_id: i64,
        result: UiResult<Vec<RewindTarget>>,
    },
    ModelCatalogLoaded {
        query: String,
        offset: usize,
        result: UiResult<ModelCatalogListResponse>,
    },
    ProviderStudioAdapterModelsLoaded {
        request_key: String,
        result: UiResult<ProviderAdapterModelsResponse>,
    },
    ProviderStudioAuthCompleted {
        request_key: String,
        result: std::result::Result<
            agena_application::provider_studio::ProviderDraftAuthActionResult,
            agena_application::provider_studio::ProviderDraftAuthError,
        >,
    },
    ProviderStudioSaved {
        provider_id: String,
        result: std::result::Result<
            agena_application::provider_studio::ProviderStudioSaveResult,
            agena_application::provider_studio::ProviderStudioSaveError,
        >,
    },
    ModelCatalogRefreshed {
        result: UiResult<()>,
    },
    ChildSessionsLoaded {
        parent_session_id: i64,
        result: UiResult<Vec<SessionResource>>,
    },
    TimelineLoaded {
        session_id: i64,
        result: UiResult<Vec<crate::app_backend::SessionTimelineEntry>>,
    },
    SessionRewound {
        session_id: i64,
        message_text: String,
        target: String,
        result: UiResult<SessionExecutionResource>,
    },
    /// Pushed by the unified event bus (`app_backend::live_events::subscribe_session_events`).
    /// Callers receive each domain event in real time, with a hint about
    /// whether a refresh is needed.
    SessionEventArrived {
        session_id: i64,
        live: LiveEvent,
    },
    /// Result of a `request_cancel_run` call. We always treat the in-flight
    /// run as gone when this lands, regardless of success — the user has
    /// already signalled cancel intent.
    RunCancelled {
        session_id: i64,
        result: UiResult<()>,
    },
    StatusLineUpdated {
        output: Option<String>,
    },
    /// A `/side` fork was created and the user was switched into it.
    SideSessionOpened {
        session_id: i64,
        parent_id: i64,
    },
}

pub(super) type UiCompletion = Box<dyn FnOnce(&mut App) + Send + 'static>;
pub(super) type UiCommand = Pin<Box<dyn Future<Output = UiCompletion> + Send + 'static>>;

#[derive(Debug, Clone)]
pub(super) enum UiAction {
    CopyText { text: String, success: String },
    EditComposerExternally,
    DownloadTerminalFile { path: PathBuf },
    ExportTranscript { path: Option<PathBuf> },
    OpenPath { path: PathBuf },
    PageTranscript,
}

#[derive(Debug, Clone)]
pub(super) struct UiFailure {
    pub(super) failure: Box<agena_failure::Failure>,
}

impl UiFailure {
    pub(super) fn from_backend(error: anyhow::Error) -> Self {
        if let Some(error) = error.downcast_ref::<agena_application::ApplicationError>() {
            return Self::from_failure((*error.failure).clone());
        }
        if let Some(error) = error.downcast_ref::<agena_runtime::RuntimeConfigSettingsError>() {
            return Self::from_failure(error.failure().clone());
        }
        if let Some(error) = error.downcast_ref::<agena_runtime::SessionExecutionCommandError>() {
            return Self::from_failure(error.failure.clone());
        }
        if let Some(error) = error.downcast_ref::<agena_runtime::SessionExecutionControlError>() {
            return Self::from_failure(error.failure.clone());
        }
        if let Some(error) = error.downcast_ref::<agena_runtime::SessionQueryError>() {
            return Self::from_failure((*error.failure).clone());
        }
        Self::internal(error)
    }

    pub(super) fn from_failure(failure: agena_failure::Failure) -> Self {
        Self {
            failure: Box::new(failure),
        }
    }

    pub(super) fn internal(diagnostic: impl std::fmt::Display) -> Self {
        let failure = agena_failure::Failure::new(
            agena_failure::FailureCode::new("ui.operation_failed"),
            agena_failure::FailureCategory::Internal,
            agena_failure::FailureResponsibility::System,
            agena_failure::RetryDirective::Unknown,
            agena_failure::RecoveryDirective::Retry,
            agena_failure::FailureImpact::RequestRejected,
            agena_failure::UserPresentation::new(
                "ui-operation-failed",
                "The terminal interface could not finish this action.",
            ),
        );
        tracing::error!(
            failure_id = %failure.id,
            diagnostic = %diagnostic,
            "terminal UI operation failed"
        );
        Self {
            failure: Box::new(failure),
        }
    }

    pub(super) fn message(message: impl Into<String>) -> Self {
        Self {
            failure: Box::new(agena_failure::Failure::new(
                agena_failure::FailureCode::new("ui.invalid_action"),
                agena_failure::FailureCategory::InvalidInput,
                agena_failure::FailureResponsibility::Caller,
                agena_failure::RetryDirective::CorrectInput,
                agena_failure::RecoveryDirective::None,
                agena_failure::FailureImpact::RequestRejected,
                agena_failure::UserPresentation::validated("ui-invalid-action", message.into()),
            )),
        }
    }

    pub(super) fn invalid_with_diagnostic(
        message: &'static str,
        diagnostic: impl std::fmt::Display,
    ) -> Self {
        let failure = agena_failure::Failure::new(
            agena_failure::FailureCode::new("ui.invalid_action"),
            agena_failure::FailureCategory::InvalidInput,
            agena_failure::FailureResponsibility::Caller,
            agena_failure::RetryDirective::CorrectInput,
            agena_failure::RecoveryDirective::None,
            agena_failure::FailureImpact::RequestRejected,
            agena_failure::UserPresentation::new("ui-invalid-action", message),
        );
        tracing::warn!(
            failure_id = %failure.id,
            diagnostic = %diagnostic,
            "terminal UI input was invalid"
        );
        Self {
            failure: Box::new(failure),
        }
    }
}

impl std::fmt::Display for UiFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.failure.user.fallback.as_str())
    }
}

impl std::error::Error for UiFailure {}

pub(crate) enum UiErrorNotice {
    Message(String),
    Failure(UiFailure),
}

impl From<String> for UiErrorNotice {
    fn from(message: String) -> Self {
        Self::Message(message)
    }
}

impl From<&str> for UiErrorNotice {
    fn from(message: &str) -> Self {
        Self::Message(message.to_owned())
    }
}

impl From<UiFailure> for UiErrorNotice {
    fn from(failure: UiFailure) -> Self {
        Self::Failure(failure)
    }
}

pub(super) type UiResult<T> = std::result::Result<T, UiFailure>;

#[derive(Debug, Clone)]
pub(super) enum Overlay {
    TranscriptSearch(LineInputOverlay),
    SessionRename(LineInputOverlay),
    SettingsValueEdit(SettingsValueEditOverlay),
    Choice(ChoiceOverlay),
    PathBrowser(PathBrowserOverlay),
    Permission(PermissionOverlay),
    UserInputReply(UserInputOverlay),
    Confirm(ConfirmOverlay),
    SessionSearch(SessionSearchOverlay),
    Timeline(TimelineOverlay),
    ProviderStudio(Box<ProviderStudioOverlay>),
    ModelCatalogStudio(ModelCatalogStudioOverlay),
}

#[derive(Debug, Clone)]
pub(super) enum Route {
    Main,
    Usage(UsageDashboardState),
    Activities(ActivitiesState),
    PlanViewer(PlanViewerState),
    SettingsStudio(SettingsStudioOverlay),
    ClientVersionsStudio(SettingsStudioOverlay),
    PermissionStudio(PermissionStudioOverlay),
    PermissionRuleStudio(PermissionRuleStudioOverlay),
    SessionSearch(SessionSearchOverlay),
    CommandPalette(CommandPaletteOverlay),
    SkillPicker(SkillPickerOverlay),
    SkillStudio(SkillStudioOverlay),
    SessionNavigation(SessionNavigationOverlay),
    SelectionPicker(SelectionPickerOverlay),
    SessionModelChooser(SessionModelChooserOverlay),
    Timeline(TimelineOverlay),
    PluginWorkbench(Box<PluginWorkbenchOverlay>),
    ProviderStudio(Box<ProviderStudioOverlay>),
    ModelCatalogStudio(ModelCatalogStudioOverlay),
}

#[derive(Debug, Clone)]
pub(super) struct UsageDashboardState {
    pub(super) period: UsagePeriod,
    pub(super) presentation: UsageDashboardPresentation,
    pub(super) available_providers: Vec<String>,
    pub(super) available_models: Vec<(String, String)>,
    pub(super) data: Option<UsageDashboardData>,
    pub(super) loading: bool,
    pub(super) request_id: u64,
    pub(super) error: Option<String>,
}

impl UsageDashboardState {
    pub(super) fn new(period: UsagePeriod) -> Self {
        Self {
            period,
            presentation: UsageDashboardPresentation::new(),
            available_providers: Vec::new(),
            available_models: Vec::new(),
            data: None,
            loading: false,
            request_id: 0,
            error: None,
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct ActivitiesState {
    pub(super) presentation: agena_tui::activities::ActivitiesPresentation,
    pub(super) rows: Vec<agena_tui::activities::ActivitiesRow>,
    pub(super) log_tail: Option<agena_tui::activities::ActivitiesLogTail>,
    pub(super) log_error: Option<String>,
    /// In-flight list refresh. Mutating actions use a separate gate so a
    /// late list response cannot make stop/dismiss/clear appear completed.
    pub(super) loading: bool,
    pub(super) mutation_loading: bool,
    pub(super) error: Option<String>,
    pub(super) request_id: u64,
    pub(super) mutation_request_id: u64,
    pub(super) log_request_id: u64,
    pub(super) log_request_since: u64,
    pub(super) log_activity_id: Option<String>,
    pub(super) log_loading: bool,
    pub(super) last_refresh_at: Instant,
    pub(super) last_log_refresh_at: Instant,
}

impl ActivitiesState {
    pub(super) fn new() -> Self {
        Self {
            presentation: agena_tui::activities::ActivitiesPresentation::default(),
            rows: Vec::new(),
            log_tail: None,
            log_error: None,
            loading: false,
            mutation_loading: false,
            error: None,
            request_id: 0,
            mutation_request_id: 0,
            log_request_id: 0,
            log_request_since: 0,
            log_activity_id: None,
            log_loading: false,
            last_refresh_at: Instant::now(),
            last_log_refresh_at: Instant::now(),
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct PlanViewerState {
    pub(super) presentation: agena_tui::plan_viewer::PlanViewerPresentation,
    pub(super) loading: bool,
    pub(super) error: Option<String>,
    pub(super) summary: Option<String>,
    pub(super) markdown: Option<String>,
    pub(super) autorun: Option<bool>,
    pub(super) request_id: u64,
    pub(super) toggle_request_id: u64,
}

impl PlanViewerState {
    pub(super) fn new() -> Self {
        Self {
            presentation: agena_tui::plan_viewer::PlanViewerPresentation::new(),
            loading: false,
            error: None,
            summary: None,
            markdown: None,
            autorun: None,
            request_id: 0,
            toggle_request_id: 0,
        }
    }
}

/// Structured payload projected from the `agena.plan` `get` UI tool for the
/// plan viewer route.
#[derive(Debug, Clone)]
pub(super) struct PlanViewerData {
    pub(super) markdown: String,
    pub(super) autorun: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DialogHost {
    Route,
    Overlay,
}

pub(super) type LineInputOverlay = InputDialogState<()>;

pub(super) use agena_tui::help::{HelpOverlay, HelpOverlayKind};

pub(super) use agena_tui_components::{
    HelpDialogEntry as HelpEntry, HelpDialogSection as HelpSection,
};

#[cfg(test)]
mod ui_failure_tests {
    use super::UiFailure;

    #[test]
    fn backend_failure_survives_anyhow_context_without_ui_rewrapping() {
        let failure = agena_failure::Failure::new(
            agena_failure::FailureCode::new("tool.execution_failed"),
            agena_failure::FailureCategory::DependencyUnavailable,
            agena_failure::FailureResponsibility::Dependency,
            agena_failure::RetryDirective::AfterUserAction,
            agena_failure::RecoveryDirective::Retry,
            agena_failure::FailureImpact::OperationFailed,
            agena_failure::UserPresentation::validated(
                "tool-execution-failed",
                "Filesystem write failed because the target changed.",
            ),
        );
        let expected_id = failure.id;
        let error = anyhow::Error::new(agena_application::ApplicationError::from_failure(failure))
            .context("failed to submit user message");

        let projected = UiFailure::from_backend(error);

        assert_eq!(projected.failure.id, expected_id);
        assert_eq!(projected.failure.code.as_str(), "tool.execution_failed");
        assert_eq!(
            projected.failure.user.fallback,
            "Filesystem write failed because the target changed."
        );
    }

    #[test]
    fn settings_validation_message_survives_backend_context_rewrap() {
        // A config settings edit that fails validation carries the real
        // user-visible message on `RuntimeConfigSettingsError`. The backend
        // wraps it with anyhow context; UiFailure::from_backend must recover
        // the structured failure instead of falling back to the generic
        // "terminal interface" message.
        let error = agena_runtime::RuntimeConfigSettingsError::invalid_input(
            "providers.default `ghost` references unknown provider",
        );
        let expected = error.failure().user.fallback.clone();
        let wrapped = anyhow::Error::new(error).context("failed to set config setting");

        let projected = UiFailure::from_backend(wrapped);

        assert!(!projected.failure.is_unexpected());
        assert_eq!(projected.failure.user.fallback, expected);
        assert_ne!(
            projected.failure.user.fallback,
            "The terminal interface could not finish this action."
        );
    }
}

#[cfg(test)]
mod terminal_integration_state_tests {
    use super::TerminalIntegrationState;
    use agena_tui_platform::terminal::integration::NotificationMethod;

    #[test]
    fn title_due_is_marked_until_emitted() {
        let mut state = TerminalIntegrationState::default();
        assert!(!state.title_due());
        assert!(state.last_title().is_none());

        state.mark_title_pending();
        assert!(state.title_due());

        state.note_title_emitted("fix login".to_owned());
        assert!(!state.title_due());
        assert_eq!(state.last_title(), Some("fix login"));
    }

    #[test]
    fn a_burst_of_notifications_coalesces_into_one_alert() {
        let mut state = TerminalIntegrationState::default();
        state.queue_notification(NotificationMethod::Bell);
        state.queue_notification(NotificationMethod::Osc9);
        state.queue_notification(NotificationMethod::Bell);

        // The most recently queued method wins; the queue drains empty.
        assert_eq!(state.take_notification(), Some(NotificationMethod::Bell));
        assert_eq!(state.take_notification(), None);
    }

    #[test]
    fn a_plugin_notify_intent_fires_exactly_once() {
        let mut state = TerminalIntegrationState::default();
        assert!(state.notify_consumed_once("\"done\""));
        assert!(!state.notify_consumed_once("\"done\""));
        assert!(state.notify_consumed_once("\"blocked\""));
    }
}
