pub mod confirm_state;
pub mod dashboard;
pub mod dashboard_selection;
pub mod decision_dialog;
pub mod detail_dialog;
pub mod detail_text;
pub mod editor;
pub mod editor_dialog;
pub mod editor_panel;
pub mod editor_preview_dialog;
pub mod editor_state;
pub mod frame;
pub mod input_dialog;
pub mod input_state;
pub mod keymap;
pub mod layout;
pub mod list_workbench_state;
pub mod panels;
pub mod question_flow;
pub mod question_flow_dialog;
pub mod scroll_state;
pub mod search_list;
pub mod search_panels;
pub mod sectioned_list;
pub mod selectable_list;
pub mod selection;
pub mod stacked_dialog;
pub mod suggestion_state;
pub mod suggestions;
pub mod surface;
pub mod text;
pub mod text_dialog;
pub mod theme;
pub mod titles;
pub mod workbench;

pub use confirm_state::ConfirmDialogState;
pub use dashboard::{
    DashboardDetailOverlaySpec, DashboardLeadPanelSpec, DashboardListPanelHeight,
    DashboardListPanelState, DashboardSplitPanelsSpec, DashboardTextPanelHeight,
    DashboardTextSection, DashboardWorkbenchOverlaySpec, DashboardWorkbenchSpec,
    render_dashboard_workbench, render_dashboard_workbench_dialog,
};
pub use dashboard_selection::DashboardSelectionState;
pub use decision_dialog::{DecisionDialogSpec, render_decision_dialog};
pub use detail_dialog::{DetailTextDialogSpec, render_detail_text_dialog};
pub use detail_text::{
    DetailDocument, DetailTextLine, DetailTextSpec, build_detail_document, build_detail_text,
    build_detail_text_plain, detail_row_display_text,
};
pub use editor::{Editor, EditorView};
pub use editor_dialog::{
    EditorDialogSpec, render_editor_dialog, render_overlay_line_input_dialog,
    render_workbench_editor_dialog,
};
pub use editor_panel::{EditorPanelRenderResult, EditorPanelSpec, render_editor_panel};
pub use editor_preview_dialog::{
    EditorPreviewDialogSpec, EditorPreviewHelpSpec, render_editor_preview_dialog,
};
pub use editor_state::{EditorDialogKeyResult, EditorDialogState, drive_editor_dialog_key};
pub use frame::{FramedSurface, FramedSurfaceSpec, render_framed_surface};
pub use input_state::{InputDialogKeyResult, InputDialogState, drive_input_dialog_key};
pub use keymap::{InputDialogAction, NavigationAction, input_dialog_action, navigation_action};
pub use layout::{
    SurfaceMode, VerticalSectionSize, adaptive_detail_split, adaptive_modal_height,
    adaptive_modal_width, bordered_paragraph_height, editor_input_panel_height,
    estimated_horizontal_panel_widths, framed_overlay_height, framed_sections_target_height,
    inset_rect, list_panel_height, optional_overlay_text_height, overlay_text_height,
    preferred_overlay_rect, should_stack_detail_layout, split_vertical_sections,
    top_aligned_panel_rect, top_aligned_vertical_areas, vertical_sections_base_height,
    wrapped_text_height,
};
pub use list_workbench_state::ListWorkbenchState;
pub use panels::{
    BoundedListPanelHeight, ListPanelHeightResolver, ListPanelSpec, ListPanelState, TextPanelSpec,
    TwoLineListItemSpec, build_accented_two_line_list_item, build_detail_two_line_list_item,
    build_two_line_list_item, render_list_panel, render_list_panel_state, render_text_panel,
};
pub use question_flow::{QuestionFlowScreen, QuestionFlowState};
pub use question_flow_dialog::{
    QuestionFlowCustomInputSpec, QuestionFlowDialogMode, QuestionFlowDialogSpec,
    render_question_flow_dialog,
};
pub use scroll_state::ScrollState;
pub use search_list::{
    SearchInputKeyResult, SearchListClearAction, SearchListCustomValue, SearchListDialogSpec,
    SearchListDialogState, SearchListInput, SearchListItem, SearchListNoCustom, SearchListOverlay,
    SearchListOverlayConfig, SearchListRow, refresh_search_list_overlay, render_search_list_dialog,
};
pub use search_panels::{
    SearchPanelsDialogSpec, SearchPanelsDialogState, SearchPanelsOverlay,
    refresh_search_panels_overlay, render_search_panels_dialog,
};
pub use sectioned_list::{SectionedListFocus, SectionedListSection, SectionedListState};
pub use selectable_list::SelectableListState;
pub use selection::{
    SelectionCursor, clamp_selected_index, move_selected_index, move_selected_index_end,
    move_selected_index_home, move_selected_index_page,
};
pub use stacked_dialog::{
    EditorSection, ListPanelSection, ParagraphSection, StackedDialogRenderResult,
    StackedDialogSection, StackedDialogSectionHeight, StackedDialogSpec, TextPanelSection,
    render_stacked_dialog,
};
pub use suggestion_state::{QuerySuggestionState, SuggestionPopupState};
pub use suggestions::{
    QuerySuggestionPopupSpec, SuggestionPopupItem, SuggestionPopupSpec,
    render_query_suggestion_popup, render_suggestion_popup,
};
pub use surface::{
    ComposerEditorSurfaceSpec, ComposerSurfaceLayout, HeaderBodyFooterLayout,
    HeaderBodyFooterTextSurfaceSpec, layout_composer_surface, layout_header_body_footer_surface,
    pane_header_height, render_composer_editor_surface, render_header_body_footer_text_surface,
};
pub use text::{
    HeaderRowSpec, WrappedTextSpec, bordered_text_height, build_wrapped_text_lines,
    format_key_value_segment, join_inline_segments, line_plain_text, render_header_row,
    render_wrapped_text, text_plain_text, trim_empty_line_edges, truncate_display_text,
    truncate_display_text_middle, truncate_display_text_with_suffix, wrapped_lines_height,
    wrapped_text_height_for_text,
};
pub use text_dialog::{LineTextDialogSpec, TextDialogLine, render_line_text_dialog};
pub use theme::{ColorScheme, TerminalRgb, ThemeOverrides, ThemePalette};
pub use titles::title_with_summary;
pub use workbench::{
    ListWorkbenchDialogSpec, ListWorkbenchPanelState, SectionedWorkbenchDialogSpec,
    WorkbenchOverlayDialogSpec, WorkbenchOverlaySource, WorkbenchTextSection,
    render_list_workbench_dialog, render_sectioned_workbench_dialog,
};
