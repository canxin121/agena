use super::{
    agent_profile_scope_label_localized, agent_profile_storage_label_localized,
    agent_studio_item_detail_text, agent_studio_overview_text,
};
use agena_tui_components::{
    BoundedListPanelHeight, ComposerEditorSurfaceSpec, DashboardDetailOverlaySpec,
    DashboardLeadPanelSpec, DashboardListPanelHeight, DashboardListPanelState,
    DashboardSplitPanelsSpec, DashboardTextPanelHeight, DashboardTextSection,
    DashboardWorkbenchOverlaySpec, DashboardWorkbenchSpec, DetailTextDialogSpec, DetailTextLine,
    DetailTextSpec, EditorDialogSpec, FramedSurfaceSpec, HeaderBodyFooterTextSurfaceSpec,
    HeaderRowSpec, LineTextDialogSpec, ListPanelSection, ListPanelSpec, ListWorkbenchDialogSpec,
    ListWorkbenchPanelState, ParagraphSection, QuerySuggestionPopupSpec,
    QuestionFlowCustomInputSpec, QuestionFlowDialogMode, QuestionFlowDialogSpec,
    SearchListDialogSpec, SearchPanelsDialogSpec, SearchPanelsDialogState, StackedDialogSection,
    StackedDialogSectionHeight, StackedDialogSpec, SuggestionPopupItem, SuggestionPopupSpec,
    SurfaceMode, TextDialogLine, TextPanelSection, TextPanelSpec, VerticalSectionSize,
    WorkbenchOverlayDialogSpec, WorkbenchTextSection, WrappedTextSpec, adaptive_detail_split,
    adaptive_modal_width, build_accented_two_line_list_item, build_detail_two_line_list_item,
    build_wrapped_text_lines, format_key_value_segment, inset_rect, join_inline_segments,
    layout_composer_surface, layout_header_body_footer_surface, list_panel_height,
    pane_header_height, render_composer_editor_surface, render_dashboard_workbench_dialog,
    render_editor_dialog, render_framed_surface, render_header_body_footer_text_surface,
    render_header_row, render_line_text_dialog, render_list_panel, render_list_workbench_dialog,
    render_overlay_line_input_dialog, render_query_suggestion_popup, render_question_flow_dialog,
    render_search_list_dialog, render_search_panels_dialog, render_stacked_dialog,
    render_suggestion_popup, render_text_panel, render_wrapped_text, split_vertical_sections,
    truncate_display_text, wrapped_text_height_for_text,
};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, BorderType, Borders, Clear, ListItem, Paragraph, Wrap},
};
use tui_markdown::from_str as markdown_to_text;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::{
    ComposerItem, I18n, PermissionAction, PermissionOverlay, PermissionOverlayPage,
    PermissionRequest, PermissionScope,
};

mod view_catalog_helpers;
mod view_help;
mod view_layout;
mod view_main;
mod view_overlays;
mod view_permission_helpers;
mod view_settings_helpers;
mod view_studio;
mod view_usage;
mod view_user_input_helpers;

pub(super) use self::view_catalog_helpers::*;
pub(super) use self::view_permission_helpers::*;
pub(super) use self::view_settings_helpers::*;
pub(super) use self::view_user_input_helpers::*;

#[cfg(test)]
mod composer_item_summary_tests {
    use super::{ComposerItem, composer_item_needs_summary_chip};
    use crate::app::{StagedAttachment, StagedPaste};
    use std::path::PathBuf;

    #[test]
    fn large_pastes_are_shown_only_by_their_inline_placeholder() {
        let paste = ComposerItem::LargePaste(StagedPaste {
            placeholder: "[paste 1001 chars]".to_string(),
            label: "paste 1001 chars".to_string(),
            text: "x".repeat(1001),
        });
        let attachment = ComposerItem::Attachment(StagedAttachment {
            path: PathBuf::from("notes.txt"),
            placeholder: "[notes.txt]".to_string(),
            label: "notes.txt".to_string(),
            is_temp: false,
        });

        assert!(!composer_item_needs_summary_chip(&paste));
        assert!(composer_item_needs_summary_chip(&attachment));
    }
}

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
    use super::{
        I18n, Line, PermissionAction, PermissionOverlay, PermissionOverlayPage, PermissionRequest,
        PermissionScope, permission_overlay_body_lines,
    };
    use crate::app::PermissionOverlayDetailsReturn;
    use agena::permission::{DecisionTraceStep, PermissionRiskLevel, PolicySourceKind};
    use agena_tui_components::SelectionCursor;
    use chrono::Utc;

    fn overlay(action: PermissionAction) -> PermissionOverlay {
        PermissionOverlay {
            session_id: 99,
            request: PermissionRequest {
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
                risk: PermissionRiskLevel::Medium,
                trace: vec![DecisionTraceStep {
                    source_kind: PolicySourceKind::StaticPolicy,
                    source: Some("static_policy".to_string()),
                    scope: None,
                    operator: None,
                    summary: "confirmation required".to_string(),
                }],
                created_at: Utc::now(),
            },
            page: PermissionOverlayPage::Action,
            selection: SelectionCursor::default(),
        }
    }

    fn rendered(lines: Vec<Line<'static>>) -> String {
        lines
            .iter()
            .flat_map(|line| line.spans.iter())
            .map(|span| span.content.as_ref())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn overview_focuses_on_the_approved_target_and_hides_audit_noise() {
        let i18n = I18n::default();
        let dialog = overlay(PermissionAction::Tool {
            tool_name: "agena.tools.tags".to_string(),
            qualifier: Some("workspace=/repo".to_string()),
        });
        let text = rendered(permission_overlay_body_lines(&i18n, &dialog));

        assert!(text.contains("agena.tools.tags"));
        assert!(text.contains("workspace=/repo"));
        assert!(text.contains("The tool will update the current workspace."));
        assert!(!text.contains("call_sensitive_request"));
        assert!(!text.contains("static_policy"));
        assert!(!text.contains("99"));
    }

    #[test]
    fn network_overview_shows_the_full_requested_url() {
        let i18n = I18n::default();
        let dialog = overlay(PermissionAction::NetworkAccess {
            target: "https://api.example.test/v1/private?scope=write".to_string(),
            host: "api.example.test".to_string(),
            port: Some(443),
        });
        let text = rendered(permission_overlay_body_lines(&i18n, &dialog));

        assert!(text.contains("https://api.example.test/v1/private?scope=write"));
        assert!(text.contains("api.example.test:443"));
    }

    #[test]
    fn details_are_available_only_when_explicitly_requested() {
        let i18n = I18n::default();
        let mut dialog = overlay(PermissionAction::Tool {
            tool_name: "shell".to_string(),
            qualifier: None,
        });
        dialog.page = PermissionOverlayPage::Details(PermissionOverlayDetailsReturn::Action);
        let text = rendered(permission_overlay_body_lines(&i18n, &dialog));

        assert!(text.contains("call_sensitive_request"));
        assert!(text.contains("static_policy"));
        assert!(text.contains("confirmation required"));
        assert!(!text.contains("99"));
    }
}
use crate::app::{
    AgentStudioOverlay, App, CatalogModelResource, ChoiceOverlay, ConfirmOverlay,
    FileAttachOverlay, FlashLevel, Focus, Frame, HelpEntry, HelpOverlay, LayoutCache,
    MAX_FILE_MENTION_SUGGESTIONS, MAX_PROMPT_HISTORY_SEARCH_RESULTS, MAX_SLASH_COMMAND_SUGGESTIONS,
    ModelCatalogStudioOverlay, Overlay, PathBrowserOverlay, PermissionRuleStudioOverlay,
    PermissionStudioAction, PermissionStudioFocus, PermissionStudioItem,
    PermissionStudioModeTarget, PermissionStudioOverlay, PermissionStudioPaneFocus,
    PermissionStudioSection, PermissionStudioSectionId, PickerOverlay,
    ProviderDraftSecretSourceKind, ProviderStudioField, ProviderStudioFocus, ProviderStudioOverlay,
    QuestionFlowScreen, Rect, Route, SessionModelChooserOverlay, SessionSearchOverlay,
    SettingsPickerAction, SettingsStudioFocus, SettingsStudioItem, SettingsStudioOverlay,
    SettingsStudioSection, SettingsStudioSectionId, TimelineOverlay, UserInputAnswerDraft,
    UserInputOverlay, UserInputQuestion, UserInputRequest, build_detail_text, find_search_ranges,
    format_tokens_k, max, min, pending_interactive_counts_for_execution, permission_action_label,
    permission_overlay_choices, permission_overlay_footer, permission_overlay_title,
    permission_related_actions_for_display, permission_requested_actions_for_display,
    permission_risk_label, permission_rule_draft_label, permission_rule_mode_label,
    permission_rule_path_access_kind_display, permission_rule_studio_detail_text,
    permission_rule_subject_kind_name, permission_trace_step_label, provider_draft_auth_mode_label,
    provider_draft_auth_subtype_label, provider_model_config_field_display,
    provider_model_config_field_editable, provider_model_config_field_label,
    provider_model_config_fields, provider_studio_adapter_list_detail,
    provider_studio_adapter_selectable, provider_studio_auth_login_kind_label,
    provider_studio_auth_state_lines, provider_studio_detail_fields,
    provider_studio_field_editable, provider_studio_field_label, provider_studio_main_field_value,
    provider_studio_model_list_detail, provider_studio_model_selected,
    provider_studio_selected_adapter_models, provider_studio_visible_fields,
    sanitize_terminal_text, ui_text, user_input_answer_values, user_input_question_label,
};
