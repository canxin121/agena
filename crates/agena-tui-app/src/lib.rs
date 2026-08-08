//! # agena-tui-app
//!
//! Application layer of the Agena terminal UI.
//!
//! This crate turns the reusable widgets from [`agena_tui_components`] into a
//! working interactive application: it owns session state, composer state,
//! permission prompts, provider/model switching, settings, overlays, the plan
//! viewer, and the transcript, and drives the runtime through the shared API
//! command surface.
//!
//! ## Public surface
//!
//! - [`App`] — the main application state machine.
//! - [`LaunchOptions`] — configuration for starting the TUI.
//! - [`tui_config_from_preferences`] — translate persisted UI preferences
//!   into a terminal style/theme configuration.

#![allow(clippy::items_after_test_module)]

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
    pagination::PaginatedResponse,
    resource::{
        PendingInteractiveRequest, PendingInteractiveRequestResource, PermissionRuleResource,
        ProviderAdapterModelsResource, ProviderAdapterModelsResponse, ProviderModelResource,
        ProviderSummaryResource, RunOptions, SessionExecutionContextResource,
        SessionExecutionResource, SessionResource,
    },
};
#[cfg(test)]
use agena_api::{
    message_part::MessagePartResource,
    resource::{MessageResource, MessageStatus},
};
use agena_application::dto::{
    ConfigJsonSources, TuiColorSchemeResource, TuiGraphicsModeResource, TuiPreferencesResource,
};
#[cfg(test)]
use agena_domain::ExecutionStatus;
use agena_domain::Model as ProviderModel;
use agena_domain::ModelRef;
use agena_domain::PermissionRequest;
use agena_domain::get_json_path;
use agena_domain::{
    PathAccessModes, PathAccessRuleConfig, PermissionAction, PermissionConfig, PermissionMode,
    PermissionReplyKind, PermissionScope, ToolPermissionRules, UserInputReplyKind,
};
use agena_domain::{UserInputQuestion, UserInputReply, UserInputRequest};
use agena_plugin_sdk::AttachmentKind;
use agena_provider::CredentialIssuer;
use agena_tui::permission_prompt::{
    PermissionPromptDecision, PermissionPromptEffect, PermissionPromptPage,
    PermissionPromptPresentation,
};
use agena_tui::presentation_config::{ColorSchemePreference, TuiConfig};
use agena_tui::terminal_graphics::GraphicsMode;
use agena_tui_settings::{
    SettingsStudioFocus, SettingsStudioItem, SettingsStudioPresentation, SettingsStudioSection,
    SettingsStudioSectionId, SettingsStudioSourceRow,
};
use anyhow::Result;
use chrono::{DateTime, Local, Utc};
use crossterm::event::{Event, KeyEvent, KeyEventKind, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Style},
};
use serde_json::{Map as JsonMap, Value as JsonValue};
use tokio::{sync::mpsc::unbounded_channel, time::interval};
use unicode_width::UnicodeWidthChar;

#[cfg(test)]
use agena_api::resource::MessageRole;

use agena_tui_backend::{
    Backend, InspectorRow, LiveEvent, ProviderConfigDraft, ProviderDraftAdapterRule,
    ProviderDraftAuthKind, ProviderDraftInteractiveLoginKind, ProviderDraftSecretSourceKind,
    SessionPermissionStudioState, SessionRefresh,
};

mod commands;
mod composer_queue;
#[cfg(test)]
mod keymap_contract_tests;

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
        Self::text_part_with_flags(id, message_id, created_at, status, text, false)
    }

    pub(crate) fn text_part_with_flags(
        id: i64,
        message_id: i64,
        created_at: DateTime<Utc>,
        status: ExecutionStatus,
        text: impl Into<String>,
        synthetic: bool,
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
            activity_id: None,
            segment_id: Some(agena_domain::TextSegmentId::new()),
            operation_id: None,
            created_at,
            content: Some(agena_api::message_part::MessagePartDetailResource::Text(
                agena_api::message_part::MessageTextPartResource {
                    text: text.into(),
                    synthetic,
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
            kind: agena_api::message_part::MessagePartKindResource::Activity,
            name: None,
            summary: None,
            has_detail: true,
            activity_id: Some(agena_domain::ActivityId::new()),
            segment_id: None,
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
        ExecutionStatus::PolicyDenied => {
            agena_api::message_part::PartExecutionStatusResource::PolicyDenied
        }
        ExecutionStatus::UserDeclined => {
            agena_api::message_part::PartExecutionStatusResource::UserDeclined
        }
        ExecutionStatus::CapabilityUnavailable => {
            agena_api::message_part::PartExecutionStatusResource::CapabilityUnavailable
        }
        ExecutionStatus::ToolUnavailable => {
            agena_api::message_part::PartExecutionStatusResource::ToolUnavailable
        }
        ExecutionStatus::Failed => agena_api::message_part::PartExecutionStatusResource::Failed,
        ExecutionStatus::Cancelled => {
            agena_api::message_part::PartExecutionStatusResource::Cancelled
        }
    }
}
use crate::commands::{CommandId, CommandSpec};
use crate::composer_queue::ComposerQueue;
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
use agena_tui_platform::clipboard::{
    ClipboardCopyMethod, ClipboardTextError, normalize_pasted_path, set_clipboard_text,
};
use agena_tui_platform::external_editor::{edit_text, open_path};
use agena_tui_platform::external_pager::page_text;
use agena_tui_platform::terminal::TerminalRuntime;
use agena_tui_platform::terminal_transfer::{download_providers, request_download};

mod app_activities;
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
mod app_plan_viewer;
mod app_provider_runtime;
mod app_provider_text;
mod app_session_events;
mod app_session_helpers;
mod app_session_input;
mod app_session_interactive;
mod app_settings;
mod app_settings_choices;
mod app_settings_helpers;
mod app_skill_picker;
mod app_skill_studio;
mod app_status_context;
mod app_studio_overlays;
mod app_studio_state_impls;
mod app_terminal_integration;
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
mod notifications;
mod plugin_workbench;
mod provider_studio;
mod run_options_state;
mod state_store_impls;
mod surface_selection;
mod transcript_state;
mod view;

pub fn tui_config_from_preferences(ui: &TuiPreferencesResource) -> TuiConfig {
    TuiConfig {
        theme: ui.theme.clone(),
        color_scheme: match ui.color_scheme {
            TuiColorSchemeResource::Auto => ColorSchemePreference::Auto,
            TuiColorSchemeResource::Dark => ColorSchemePreference::Dark,
            TuiColorSchemeResource::Light => ColorSchemePreference::Light,
        },
        graphics: match ui.graphics {
            TuiGraphicsModeResource::Auto => GraphicsMode::Auto,
            TuiGraphicsModeResource::Native => GraphicsMode::Native,
            TuiGraphicsModeResource::Unicode => GraphicsMode::Unicode,
        },
        ..Default::default()
    }
}

use self::app_choice_helpers::*;
use self::app_command_helpers::*;
use self::app_composer_helpers::*;
use self::app_permission_display::*;
use self::app_permission_helpers::*;
use self::app_permission_studio::apply_permission_studio_entries_mode;
use self::app_provider_text::*;
use self::app_session_helpers::*;
use self::app_settings_helpers::*;
use self::app_terminal_integration::*;
use self::app_timeline_helpers::*;
use self::app_transcript_helpers::*;
pub(crate) use self::app_types::ComposerDraft;
use self::app_types::*;
pub use self::app_types::{App, LaunchOptions};
use self::app_usage::*;
pub(crate) use self::plugin_workbench::PluginWorkbenchOverlay;
use self::provider_studio::provider_auth::*;
use self::provider_studio::provider_fields::*;
use self::provider_studio::provider_selection::*;
use self::state_store_impls::*;
pub(crate) use self::surface_selection::{
    SurfaceDisplayLine, SurfaceLayout, SurfaceSelection, SurfaceSelectionKind,
    apply_cell_range_highlight, surface_selection_ranges, surface_selection_text,
};
pub(crate) use agena_tui_transcript::renderer as transcript_view;
pub(crate) use agena_tui_transcript::text as ui_text;
use agena_tui_transcript::{
    initial_search_match_index, normalize_transcript_text_selection, transcript_entries,
    transcript_node_highlight_range, transcript_node_kind_label,
    transcript_selection_scroll_position, transcript_spinner_placeholder,
    transcript_text_selection_text,
};

#[cfg(test)]
use agena_tui_transcript::{
    transcript_message_navigation_target, transcript_semantic_line_range,
    transcript_should_fall_back_to_message_navigation, transcript_vertical_line_navigation_step,
    transcript_vertical_navigation_step,
};

use self::transcript_view::{
    current_spinner_millis, refresh_spinner_line, render_entry_detailed, render_entry_export,
    render_transcript_snapshot_export_markdown, spinner_frame,
};
pub(crate) use agena_tui_transcript::sanitize_terminal_text;

#[cfg(test)]
mod tui_config_tests {
    use super::tui_config_from_preferences;
    use agena_application::dto::{
        TuiColorSchemeResource, TuiGraphicsModeResource, TuiPreferencesResource,
    };
    use agena_tui::presentation_config::ColorSchemePreference;
    use agena_tui::terminal_graphics::GraphicsMode;

    #[test]
    fn persistent_tui_preferences_are_projected_into_the_tui_model() {
        let persistent = TuiPreferencesResource {
            locale: Some("en-US".to_owned()),
            color_scheme: TuiColorSchemeResource::Light,
            graphics: TuiGraphicsModeResource::Native,
            theme: Some("ocean".to_owned()),
        };

        let config = tui_config_from_preferences(&persistent);
        assert_eq!(config.color_scheme, ColorSchemePreference::Light);
        assert_eq!(config.graphics, GraphicsMode::Native);
        assert_eq!(config.theme.as_deref(), Some("ocean"));
    }
}
