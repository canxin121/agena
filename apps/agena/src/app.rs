use std::{
    cmp::{max, min},
    collections::{BTreeMap, BTreeSet, HashSet},
    env, fs,
    ops::Range,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

use agena_api::{
    commands::UpsertPermissionRuleParams,
    message_part::{
        MessagePartDetailResource, MessagePartResource, MessageRequestPartResource,
        OperationBlockResource, OperationPartResource, PartExecutionStatusResource,
        ToolInvocationResource,
    },
    pagination::PaginatedResponse,
    resource::{
        MessageResource, MessageRole, MessageStatus, PendingInteractiveRequest,
        PendingInteractiveRequestResource, PermissionRuleResource, ProviderAdapterModelsResource,
        ProviderAdapterModelsResponse, ProviderModelResource, ProviderSummaryResource, RunOptions,
        SessionExecutionContextResource, SessionExecutionResource, SessionResource,
    },
};
use agena_application::dto::{
    ConfigJsonSources, RuntimeAgentProfileResource as AgentProfile,
    RuntimeAgentResource as AgentDescriptor, RuntimeAgentSelectionResource as AgentSelectionConfig,
};
#[cfg(test)]
use agena_domain::ExecutionStatus;
use agena_domain::Model as ProviderModel;
use agena_domain::ModelRef;
use agena_domain::PermissionRequest;
use agena_domain::get_json_path;
use agena_domain::{AgentScope, UserInputQuestion, UserInputReply, UserInputRequest};
use agena_domain::{
    NetworkPermissionConfig, PathAccessModes, PathAccessRuleConfig, PathPermissionConfig,
    PermissionAction, PermissionConfig, PermissionMode, PermissionReplyKind, PermissionScope,
    ToolPermissionConfig, ToolPermissionRules, UserInputReplyKind,
};
use agena_plugin_sdk::{AttachmentItem, AttachmentKind};
use agena_provider::CredentialIssuer;
use agena_tui::permission_prompt::{
    PermissionPromptDecision, PermissionPromptEffect, PermissionPromptPage,
    PermissionPromptPresentation,
};
use agena_tui::settings_studio::{
    SettingsStudioFocus, SettingsStudioItem, SettingsStudioPresentation, SettingsStudioSection,
    SettingsStudioSectionId, SettingsStudioSourceRow,
};
use anyhow::Result;
use chrono::{DateTime, Local, Utc};
use crossterm::event::{Event, KeyEvent, KeyEventKind, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
};
use serde_json::{Map as JsonMap, Value as JsonValue, json};
use tokio::{sync::mpsc::unbounded_channel, time::interval};
use unicode_width::UnicodeWidthChar;

use crate::attachment_source::{
    AttachmentAcquisition, AttachmentSource, Iterm2UploadSource, KittyUploadSource,
    acquire_clipboard_image, acquire_from_source,
};
use crate::backend::{
    Backend, InspectorRow, LiveEvent, ProviderConfigDraft, ProviderDraftAdapterRule,
    ProviderDraftAuthKind, ProviderDraftInteractiveLoginKind, ProviderDraftSecretSourceKind,
    ProviderNativeToolsPreset, SessionPermissionStudioState, SessionRefresh,
    provider_native_tools_config_for_preset, provider_native_tools_preset_from_config,
};

/// Test-only builders for transcript fixtures. They construct the public API
/// projection directly and deliberately do not accept Runtime message parts.
#[cfg(test)]
pub(crate) struct TranscriptFixture;

#[cfg(test)]
impl TranscriptFixture {
    pub(crate) fn text_part(
        id: i64,
        message_id: i64,
        created_at: DateTime<Utc>,
        status: ExecutionStatus,
        text: impl Into<String>,
    ) -> MessagePartResource {
        Self::text_part_with_flags(id, message_id, created_at, status, text, false, false)
    }

    pub(crate) fn text_part_with_flags(
        id: i64,
        message_id: i64,
        created_at: DateTime<Utc>,
        status: ExecutionStatus,
        text: impl Into<String>,
        synthetic: bool,
        ignored: bool,
    ) -> MessagePartResource {
        MessagePartResource {
            id,
            message_id,
            part_index: 0,
            status: fixture_part_status(status),
            kind: agena_api::message_part::MessagePartKindResource::Text,
            name: None,
            summary: None,
            has_detail: true,
            operation_id: None,
            created_at,
            content: Some(agena_api::message_part::MessagePartDetailResource::Text(
                agena_api::message_part::MessageTextPartResource {
                    text: text.into(),
                    synthetic,
                    ignored,
                },
            )),
        }
    }

    pub(crate) fn reasoning_part(
        id: i64,
        message_id: i64,
        created_at: DateTime<Utc>,
        status: ExecutionStatus,
        reasoning: agena_domain::ReasoningPart,
    ) -> MessagePartResource {
        MessagePartResource {
            id,
            message_id,
            part_index: 0,
            status: fixture_part_status(status),
            kind: agena_api::message_part::MessagePartKindResource::Reasoning,
            name: None,
            summary: None,
            has_detail: true,
            operation_id: None,
            created_at,
            content: Some(
                agena_api::message_part::MessagePartDetailResource::Reasoning(
                    agena_api::message_part::MessageReasoningPartResource {
                        summary: reasoning.summary,
                        raw_content: reasoning.raw_content,
                        encrypted_content: reasoning.encrypted_content,
                    },
                ),
            ),
        }
    }

    pub(crate) fn operation_part(
        id: i64,
        message_id: i64,
        created_at: DateTime<Utc>,
        status: ExecutionStatus,
        operation: OperationPartResource,
    ) -> MessagePartResource {
        MessagePartResource {
            id,
            message_id,
            part_index: 0,
            status: fixture_part_status(status),
            kind: agena_api::message_part::MessagePartKindResource::Operation,
            name: None,
            summary: None,
            has_detail: true,
            operation_id: None,
            created_at,
            content: Some(
                agena_api::message_part::MessagePartDetailResource::Operation(Box::new(operation)),
            ),
        }
    }

    pub(crate) fn permission_request_part(
        id: i64,
        message_id: i64,
        created_at: DateTime<Utc>,
        status: ExecutionStatus,
        request: agena_api::resource::PermissionRequest,
    ) -> MessagePartResource {
        MessagePartResource {
            id,
            message_id,
            part_index: 0,
            status: fixture_part_status(status),
            kind: agena_api::message_part::MessagePartKindResource::Request,
            name: None,
            summary: None,
            has_detail: true,
            operation_id: None,
            created_at,
            content: Some(agena_api::message_part::MessagePartDetailResource::Request(
                Box::new(MessageRequestPartResource::Permission {
                    request,
                    reply: None,
                }),
            )),
        }
    }
}

#[cfg(test)]
const fn fixture_part_status(
    status: ExecutionStatus,
) -> agena_api::message_part::PartExecutionStatusResource {
    match status {
        ExecutionStatus::Pending => agena_api::message_part::PartExecutionStatusResource::Pending,
        ExecutionStatus::InProgress => {
            agena_api::message_part::PartExecutionStatusResource::InProgress
        }
        ExecutionStatus::Completed => {
            agena_api::message_part::PartExecutionStatusResource::Completed
        }
        ExecutionStatus::Failed => agena_api::message_part::PartExecutionStatusResource::Failed,
        ExecutionStatus::Cancelled => {
            agena_api::message_part::PartExecutionStatusResource::Cancelled
        }
    }
}
use crate::clipboard::{
    ClipboardCopyMethod, ClipboardTextError, normalize_pasted_path, pasted_image_format,
    set_clipboard_text,
};
use crate::commands::{self, CommandId, CommandSpec};
use crate::composer_queue::{ComposerQueue, QueuePriority, QueuedMessage};
use crate::external_editor::{edit_text, open_path};
use crate::external_pager::page_text;
use crate::terminal::{TerminalContext, TerminalRuntime};
use crate::terminal_transfer::{download_providers, request_download};
use crate::ui_text;
use agena_application::dto::{
    CatalogModelResource, ModelCatalogListResponse, ModelCatalogResponse,
};
use agena_tui::i18n::{I18n, SUPPORTED_LOCALES};
use agena_tui_components::{
    ConfirmDialogState, DashboardSelectionState, DetailTextLine, DetailTextSpec, Editor,
    EditorDialogKeyResult, InputDialogKeyResult, SearchPickerClearAction, SearchPickerConfig,
    SearchPickerInputResult, SearchPickerPreviewMode, SearchPickerSearchMode, SectionedListState,
    SelectableListState, SelectionCursor, build_detail_document, build_detail_text,
    drive_editor_dialog_key, drive_input_dialog_key, format_key_value_segment,
    join_inline_segments,
};

mod app_choice_helpers;
mod app_command_actions;
mod app_command_helpers;
mod app_composer;
mod app_composer_helpers;
mod app_composer_state;
mod app_help;
mod app_input;
mod app_lifecycle;
mod app_mouse;
mod app_navigation;
mod app_overlays;
mod app_paste;
mod app_permission_display;
mod app_permission_helpers;
mod app_permission_studio;
mod app_permissions;
mod app_provider_runtime;
mod app_provider_text;
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
mod app_usage;
mod app_user_input;
mod composer_state_impls;
mod plugin_workbench;
mod provider_studio;
mod run_options_state;
mod state_store_impls;
mod transcript_navigation;
mod transcript_selection;
mod transcript_state;
mod transcript_view;
mod view;

use self::app_choice_helpers::*;
use self::app_command_helpers::*;
use self::app_composer_helpers::*;
use self::app_permission_display::*;
use self::app_permission_helpers::*;
use self::app_permission_studio::apply_permission_studio_entries_mode;
use self::app_provider_text::*;
use self::app_session_helpers::*;
use self::app_settings_helpers::*;
use self::app_timeline_helpers::*;
use self::app_transcript_helpers::*;
pub(crate) use self::app_types::ComposerDraft;
use self::app_types::*;
pub use self::app_types::{App, LaunchOptions};
use self::app_usage::*;
use self::plugin_workbench::*;
use self::provider_studio::provider_auth::*;
use self::provider_studio::provider_fields::*;
use self::provider_studio::provider_model_helpers::*;
use self::provider_studio::provider_selection::*;
use self::state_store_impls::*;
use self::transcript_navigation::*;
use self::transcript_selection::*;

use self::transcript_view::{
    current_spinner_millis, markdown_blocks, refresh_spinner_line, render_markdown_block,
    render_message_detailed, render_message_export, render_transcript_export_markdown,
    rewind_message_preview, sanitize_terminal_text, spinner_frame, transcript_spinner_placeholder,
};
