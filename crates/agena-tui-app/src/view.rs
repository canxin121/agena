use agena_tui_components::{
    BoundedListPanelHeight, ComposerEditorSurfaceSpec, ComposerStatusPlacement,
    DashboardDetailOverlaySpec, DashboardLeadPanelSpec, DashboardListPanelHeight,
    DashboardListPanelState, DashboardSplitPanelsSpec, DashboardTextPanelHeight,
    DashboardTextSection, DashboardWorkbenchOverlaySpec, DashboardWorkbenchSpec,
    DetailTextDialogSpec, DetailTextLine, DetailTextSpec, HeaderBodyFooterTextSurfaceSpec,
    ListWorkbenchDialogSpec, ListWorkbenchPanelState, SectionedWorkbenchDialogSpec, SurfaceMode,
    VerticalSectionSize, WorkbenchOverlayDialogSpec, WorkbenchTextSection, WrappedTextSpec,
    build_accented_two_line_list_item, build_detail_two_line_list_item, build_wrapped_text_lines,
    composer_corner_placement_left, composer_corner_placement_right,
    composer_status_placement_left, format_fixed_columns, format_key_value_segment, inset_rect,
    join_inline_segments, layout_composer_surface, layout_header_body_footer_surface,
    pane_header_height, panel_highlight_style, render_composer_editor_surface,
    render_confirm_dialog, render_dashboard_workbench_dialog,
    render_header_body_footer_text_surface, render_help_dialog, render_list_workbench_dialog,
    render_overlay_line_input_dialog, render_sectioned_workbench_dialog, render_wrapped_text,
    split_vertical_sections, truncate_display_text, workbench_navigation_width,
};
use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::Paragraph,
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::I18n;
#[cfg(test)]
use crate::permission_prompt_content;

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

#[cfg(test)]
mod permission_path_display_tests {
    use super::{
        UnicodeWidthStr, compact_permission_path_pattern, compact_permission_path_rule_label,
        ellipsize_from_start,
    };

    #[test]
    fn compact_path_rules_keep_distinguishing_suffixes_visible() {
        let generated = compact_permission_path_pattern(
            "/home/canxin/agena/projects/home-canxin-Git-ai/generated/client/**",
            34,
        );
        let fixtures = compact_permission_path_pattern(
            "/home/canxin/agena/projects/home-canxin-Git-ai/fixtures/mcp/**",
            34,
        );

        assert_ne!(generated, fixtures);
        assert!(generated.contains("generated/client/**"));
        assert!(fixtures.contains("fixtures/mcp/**"));
        assert!(generated.starts_with("/…/"));
        assert!(fixtures.starts_with("/…/"));
    }

    #[test]
    fn compact_workspace_paths_keep_the_workspace_marker_and_access_kind() {
        let label = compact_permission_path_rule_label(
            "<workspace>/very/long/shared/prefix/src/generated/**",
            "write",
            42,
        );

        assert!(label.starts_with("<workspace>/…/"));
        assert!(label.contains("src/generated/**"));
        assert!(label.ends_with(" · write"));
        assert!(UnicodeWidthStr::width(label.as_str()) <= 42);
    }

    #[test]
    fn extremely_narrow_path_columns_still_keep_the_tail() {
        assert_eq!(ellipsize_from_start("long-final-segment", 8), "…segment");
    }
}

#[cfg(test)]
mod permission_overlay_presentation_tests {
    use super::{I18n, permission_prompt_content};
    use crate::{PermissionAction, PermissionRequest, PermissionScope};
    use agena_domain::{DecisionTraceStep, PolicySourceKind};
    use agena_tui::permission_prompt::{PermissionPromptLine, PermissionPromptPresentation};
    use chrono::Utc;

    fn request(action: PermissionAction) -> PermissionRequest {
        PermissionRequest {
            request_id: "call_sensitive_request".to_string(),
            session_id: Some(99),
            action,
            related_actions: Vec::new(),
            requested_actions: Vec::new(),
            reason: "policy requires confirmation".to_string(),
            explanation: "The tool will update the current workspace.".to_string(),
            source: Some("static_policy".to_string()),
            scope: Some(PermissionScope::Workspace),
            operator: None,
            trace: vec![DecisionTraceStep {
                source_kind: PolicySourceKind::StaticPolicy,
                source: Some("static_policy".to_string()),
                scope: None,
                operator: None,
                summary: "confirmation required".to_string(),
            }],
            created_at: Utc::now(),
        }
    }

    fn rendered(lines: &[PermissionPromptLine]) -> String {
        lines
            .iter()
            .map(|line| line.markdown.as_deref().unwrap_or(line.text.as_str()))
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn overview_focuses_on_the_action_and_reason_and_hides_audit_noise() {
        let i18n = I18n::default();
        let request = request(PermissionAction::Tool {
            tool_name: "tools_tags".to_string(),
            qualifier: Some("workspace=/repo".to_string()),
        });
        let content = permission_prompt_content(&i18n, &request);
        let text = rendered(content.overview.as_slice());

        // One-line action, primary reason, and risk only.
        assert!(text.contains("tools_tags"));
        assert!(text.contains("workspace=/repo"));
        // `reason` is authoritative; the `explanation` moved behind Details.
        assert!(text.contains("policy requires confirmation"));
        assert!(!text.contains("The tool will update the current workspace."));
        assert!(!text.contains("call_sensitive_request"));
        assert!(!text.contains("static_policy"));
        assert!(!text.contains("99"));
    }

    #[test]
    fn overview_shows_concise_action_summary_while_details_carries_the_full_url() {
        let i18n = I18n::default();
        let request = request(PermissionAction::NetworkAccess {
            target: "https://api.example.test/v1/private?scope=write".to_string(),
            host: "api.example.test".to_string(),
            port: Some(443),
        });
        let content = permission_prompt_content(&i18n, &request);
        let overview = rendered(content.overview.as_slice());
        let details = rendered(content.details.as_slice());

        // Overview keeps the endpoint via the action summary.
        assert!(overview.contains("https://api.example.test/v1/private?scope=write"));
        // The host:port expansion lives in Details (locale-independent value).
        assert!(details.contains("api.example.test:443"));
        // The primary action is rendered twice only in Details (one-line summary
        // + per-field expansion); overview carries just the one-line summary.
        assert_eq!(
            overview
                .matches("https://api.example.test/v1/private?scope=write")
                .count(),
            1,
            "overview must not duplicate the endpoint across multiple fields"
        );
    }

    #[test]
    fn details_are_available_only_when_explicitly_requested() {
        let i18n = I18n::default();
        let request = request(PermissionAction::Tool {
            tool_name: "agena.shell.run".to_string(),
            qualifier: None,
        });
        let mut presentation =
            PermissionPromptPresentation::new(permission_prompt_content(&i18n, &request));
        assert!(presentation.open_details());
        let text = rendered(presentation.active_content());

        assert!(text.contains("call_sensitive_request"));
        assert!(text.contains("static_policy"));
        assert!(text.contains("confirmation required"));
        assert!(!text.contains("99"));
    }
}
use crate::{
    App, CatalogModelResource, ConfirmOverlay, Frame, LayoutCache, ModelCatalogStudioOverlay,
    Overlay, PermissionRuleStudioOverlay, PermissionStudioAction, PermissionStudioFocus,
    PermissionStudioItem, PermissionStudioModeTarget, PermissionStudioOverlay,
    PermissionStudioPaneFocus, PermissionStudioSection, PermissionStudioSectionId,
    ProviderDraftSecretSourceKind, ProviderStudioField, ProviderStudioFocus, ProviderStudioOverlay,
    Rect, Route, SettingsPickerAction, SettingsStudioFocus, SettingsStudioItem,
    SettingsStudioOverlay, build_detail_text, find_search_ranges, max, min,
    pending_interactive_counts_for_execution, permission_rule_draft_label,
    permission_rule_mode_label, permission_rule_studio_detail_text,
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
