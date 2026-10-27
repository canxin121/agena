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
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    fs,
    ops::Range,
    path::{Path, PathBuf},
    process::Stdio,
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
use agena_application::dto::ConfigJsonSources;
mod preferences;

use agena_domain::Model as ProviderModel;
use agena_domain::ModelRef;
use agena_domain::PermissionRequest;
use agena_domain::get_json_path;
use agena_domain::{
    PathAccessModes, PermissionAction, PermissionConfig, PermissionMode, PermissionReplyKind,
    PermissionScope, ToolPermissionRules, UserInputReplyKind,
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
pub use preferences::{TuiColorSchemeResource, TuiGraphicsModeResource, TuiPreferencesResource};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Style},
};
use serde_json::{Map as JsonMap, Value as JsonValue};
use tokio::{process::Command, time::interval};
use unicode_width::UnicodeWidthChar;

pub use crate::app_backend::{BackendMode, TuiBackend};
use crate::app_backend::{InspectorRow, LiveEvent, SessionPermissionStudioState, SessionRefresh};
use agena_application::provider_studio::{
    ProviderConfigDraft, ProviderDraftAdapterRule, ProviderDraftAuthKind,
    ProviderDraftInteractiveLoginKind, ProviderDraftSecretSourceKind,
};

mod commands;

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
use agena_tui_platform::terminal::TerminalRuntime;

mod app_activities;
mod app_backend;
mod app_btw;
mod app_choice_helpers;
mod app_command_actions;
mod app_command_helpers;
mod app_composer;
mod app_composer_helpers;
mod app_composer_state;
mod app_exit;
mod app_help;
mod app_hub;
mod app_input;
mod app_lifecycle;
mod app_message_queue;
mod app_mouse;
mod app_navigation;
mod app_overlays;
mod app_paste;
mod app_permission_display;
mod app_permission_helpers;
mod app_permission_studio;
mod app_permissions;
mod app_plan_document;
mod app_plan_viewer;
mod app_pointer;
mod app_provider_runtime;
mod app_provider_text;
mod app_queue_editor;
mod app_session_events;
mod app_session_helpers;
mod app_session_interactive;
mod app_session_marks;
mod app_session_plan;
mod app_session_work;
mod app_settings;
mod app_settings_choices;
mod app_settings_helpers;
mod app_status_context;
mod app_studio_overlays;
mod app_studio_state_impls;
mod app_terminal_integration;

mod app_timeline_helpers;
mod app_transcript_actions;
mod app_transcript_fold;
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
mod skills_workbench;
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
    initial_search_match_index, normalize_transcript_text_selection,
    transcript_node_highlight_range, transcript_node_kind_label,
    transcript_selection_scroll_position, transcript_spinner_placeholder,
    transcript_text_selection_text,
};

use self::transcript_view::{current_spinner_millis, refresh_spinner_line, spinner_frame};
pub(crate) use agena_tui_transcript::sanitize_terminal_text;
