use std::{
    cmp::{max, min},
    collections::{BTreeMap, BTreeSet, HashSet},
    env, fs,
    ops::Range,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

use agena::{
    agent::{
        NetworkPermissionConfig, PathAccessModes, PathAccessRuleConfig, PathPermissionConfig,
        PermissionConfig, ToolPermissionConfig, ToolPermissionRules,
    },
    agents::{AgentDescriptor, AgentFrontmatter, AgentProfile, AgentScope},
    config::get_json_path,
    event::{DomainEvent, EventKind as AgenaSessionEvent},
    message::{
        AttachmentKind, ExecutionStatus, MessagePart, MessageStatus, OperationPart, PartContent,
        ToolInvocation, UserInputQuestion, UserInputReply, UserInputReplyKind, UserInputRequest,
    },
    model::ModelRef,
    permission::{
        DecisionTraceStep, PermissionAction, PermissionMode, PermissionReplyKind,
        PermissionRequest, PermissionRiskLevel, PermissionScope, PolicySourceKind,
    },
    provider::{ProviderModel, auth::CredentialIssuer},
};
use agena_api::{
    commands::UpsertPermissionRuleParams,
    pagination::PaginatedResponse,
    resource::{
        MessageResource, MessageRole, PendingInteractiveRequest, PermissionRuleResource,
        ProviderAdapterModelsResource, ProviderAdapterModelsResponse, ProviderSummaryResource,
        RunOptions, SessionExecutionContextResource, SessionExecutionResource, SessionResource,
        SessionRunState, SessionUsageResource,
    },
};
use anyhow::Result;
use chrono::{DateTime, Local, Utc};
use crossterm::event::{Event, EventStream, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::{
    Frame, Terminal,
    backend::Backend as RatatuiBackend,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
};
use serde_json::{Map as JsonMap, Value as JsonValue, json};
use tokio::{sync::mpsc::unbounded_channel, time::interval};
use unicode_width::UnicodeWidthChar;

use crate::backend::{
    Backend, ConfigJsonSources, InspectorRow, LiveEvent, ProviderConfigDraft,
    ProviderDraftAdapterRule, ProviderDraftAuthKind, ProviderDraftInteractiveLoginKind,
    ProviderDraftSecretSourceKind, ProviderNativeToolsPreset, SessionPermissionStudioState,
    SessionRefresh, provider_native_tools_config_for_preset,
    provider_native_tools_preset_from_config,
};
use crate::clipboard::{
    ClipboardCopyMethod, normalize_pasted_path, paste_image_to_temp_png, pasted_image_format,
    set_clipboard_text,
};
use crate::commands::{self, CommandId, CommandSpec};
use crate::composer_queue::{ComposerQueue, QueuePriority, QueuedMessage};
use crate::external_editor::{edit_text, open_path};
use crate::external_pager::page_text;
use crate::i18n::{I18n, SUPPORTED_LOCALES};
use crate::iterm2;
use crate::keybindings::ComposerAction;
use crate::terminal;
use crate::ui_text;
use agena_api_server::local_api::{
    CatalogModelResource, ModelCatalogListResponse, ModelCatalogResponse,
};
use agena_tui_components::{
    ConfirmDialogState, DashboardSelectionState, DetailTextLine, DetailTextSpec, Editor,
    EditorDialogKeyResult, InputDialogKeyResult, ListWorkbenchState, QuestionFlowScreen,
    QuestionFlowState, SearchInputKeyResult, SearchListClearAction, SearchListOverlayConfig,
    SearchListRow, SearchPanelsOverlay, SectionedListState, SelectableListState, SelectionCursor,
    build_detail_document, build_detail_text, drive_editor_dialog_key, drive_input_dialog_key,
    format_key_value_segment, join_inline_segments, move_selected_index,
    refresh_search_list_overlay, refresh_search_panels_overlay,
};

mod app_choice_helpers;
mod app_command_actions;
mod app_command_helpers;
mod app_composer;
mod app_composer_helpers;
mod app_composer_state;
mod app_input;
mod app_lifecycle;
mod app_navigation;
mod app_overlays;
mod app_paste;
mod app_permission_display;
mod app_permission_helpers;
mod app_permission_studio;
mod app_permissions;
mod app_provider_runtime;
mod app_provider_text;
mod app_search_items;
mod app_session_events;
mod app_session_helpers;
mod app_session_input;
mod app_session_interactive;
mod app_settings;
mod app_settings_choices;
mod app_settings_helpers;
mod app_status_context;
mod app_studio_overlays;
mod app_studio_state_impls;
#[cfg(test)]
mod app_tests;
mod app_timeline_helpers;
mod app_transcript_actions;
mod app_transcript_helpers;
mod app_transcript_input;
mod app_types;
mod app_user_input;
mod composer_state_impls;
mod plugin_workbench;
mod provider_studio;
mod run_options_state;
mod state_store_impls;
mod transcript_navigation;
mod transcript_state;
mod transcript_view;
mod view;

use self::app_choice_helpers::*;
use self::app_command_helpers::*;
use self::app_composer_helpers::*;
use self::app_permission_display::*;
use self::app_permission_helpers::*;
use self::app_provider_text::*;
use self::app_session_helpers::*;
use self::app_settings_helpers::*;
use self::app_timeline_helpers::*;
use self::app_transcript_helpers::*;
pub(crate) use self::app_types::ComposerDraft;
use self::app_types::*;
pub use self::app_types::{App, LaunchOptions};
use self::plugin_workbench::*;
use self::provider_studio::provider_auth::*;
use self::provider_studio::provider_fields::*;
use self::provider_studio::provider_model_helpers::*;
use self::provider_studio::provider_selection::*;
use self::state_store_impls::*;
use self::transcript_navigation::*;

use self::transcript_view::{
    current_spinner_millis, push_markdown, render_message_detailed, render_message_export,
    render_transcript_export_markdown, rewind_message_preview, sanitize_terminal_text,
    spinner_frame,
};
