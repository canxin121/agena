use agena_tui_components::{
    BoundedListPanelHeight, ComposerEditorSurfaceSpec, ComposerStatusPlacement,
    DashboardDetailOverlaySpec, DashboardLeadPanelSpec, DashboardListPanelHeight,
    DashboardListPanelState, DashboardSplitPanelsSpec, DashboardTextPanelHeight,
    DashboardTextSection, DashboardWorkbenchOverlaySpec, DashboardWorkbenchSpec,
    DetailTextDialogSpec, DetailTextLine, DetailTextSpec, HeaderBodyFooterTextSurfaceSpec,
    ListWorkbenchDialogSpec, ListWorkbenchPanelState, SurfaceMode, VerticalSectionSize,
    WorkbenchOverlayDialogSpec, WorkbenchTextSection, WrappedTextSpec,
    build_accented_two_line_list_item, build_detail_two_line_list_item, build_wrapped_text_lines,
    composer_corner_placement_right, composer_status_placement_left, format_fixed_columns,
    format_key_value_segment, inset_rect, join_inline_segments, layout_composer_surface,
    layout_header_body_footer_surface, panel_highlight_style, render_composer_editor_surface,
    render_confirm_dialog, render_dashboard_workbench_dialog,
    render_header_body_footer_text_surface, render_help_dialog, render_list_workbench_dialog,
    render_overlay_line_input_dialog, render_wrapped_text, split_vertical_sections,
    truncate_display_text, workbench_navigation_width,
};
use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::Paragraph,
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::I18n;

mod view_catalog_helpers;
mod view_help;
mod view_layout;
mod view_main;
mod view_overlays;
mod view_permission_helpers;
mod view_settings_helpers;
mod view_studio;
mod view_usage;

pub(super) use self::view_catalog_helpers::*;
pub(super) use self::view_permission_helpers::*;
pub(super) use self::view_settings_helpers::*;

use crate::{
    App, CatalogModelResource, ConfirmOverlay, Frame, LayoutCache, ModelCatalogStudioOverlay,
    Overlay, PermissionRuleStudioOverlay, PermissionStudioAction, PermissionStudioFocus,
    PermissionStudioItem, PermissionStudioModeTarget, PermissionStudioOverlay,
    PermissionStudioPaneFocus, PermissionStudioSection, PermissionStudioSectionId,
    ProviderDraftSecretSourceKind, ProviderStudioField, ProviderStudioFocus, ProviderStudioOverlay,
    Rect, Route, SettingsPickerAction, SettingsStudioItem, SettingsStudioOverlay,
    build_detail_text, find_search_ranges, max, min, pending_interactive_counts_for_execution,
    permission_rule_draft_label, permission_rule_mode_label, permission_rule_studio_detail_text,
    permission_rule_subject_kind_name, provider_draft_auth_mode_label,
    provider_draft_auth_subtype_label, provider_model_config_field_display,
    provider_model_config_field_editable, provider_model_config_field_label,
    provider_model_config_fields, provider_studio_adapter_list_detail,
    provider_studio_adapter_selectable, provider_studio_auth_login_kind_label,
    provider_studio_auth_state_lines, provider_studio_detail_fields,
    provider_studio_field_editable, provider_studio_field_label, provider_studio_main_field_value,
    provider_studio_model_list_detail, provider_studio_model_selected,
    provider_studio_selected_adapter_models, provider_studio_visible_fields,
    sanitize_terminal_text,
};
