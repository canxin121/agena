//! Canonical state shared by contextual-help and diagnostic presentation.
//!
//! The final application decides which route or runtime diagnostic to show and
//! performs clipboard effects. This module owns only the TUI-facing identity
//! and lossless plain-text projection of an already-built help surface.

use agena_tui_components::{HelpDialogEntry, HelpDialogSection, HelpDialogState, ScrollState};

use crate::i18n::I18n;

/// One localized entry reference in a contextual-help key card.
pub type HelpEntrySpec = (&'static str, &'static str);
/// One localized key card in a contextual-help document.
pub type HelpSectionSpec = (&'static str, Vec<HelpEntrySpec>);

/// Contextual-help card selected by an application route or overlay adapter.
/// The card vocabulary belongs to TUI; only the route-specific selection is
/// application behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextHelpPreset {
    Sessions,
    Transcript,
    Composer,
    ComposerItems,
    PromptHistory,
    Suggestion,
    SingleLineEditor,
    MultiLineEditor,
    SearchPicker,
    ChoiceList,
    Timeline,
    Permission,
    ReadOnlyDetails,
    UserInputQuestion,
    UserInputEditor,
    UserInputReview,
    UserInputDecisionReview,
    Confirm,
    Usage,
    BasicList,
    Settings,
    ActionPane,
    PermissionRule,
    Provider,
    ProviderModel,
    ModelCatalog,
    PluginList,
    PluginDetail,
    PluginConfig,
    PluginActions,
    PluginSelection,
    PluginDrilldown,
    PluginDiff,
}

/// Returns the canonical TUI-owned key-card specs for every contextual-help
/// preset. Route and overlay selection remains an application concern, but the
/// complete static presentation catalog has no application fallback.
pub fn preset_specs(
    preset: ContextHelpPreset,
) -> (&'static str, Vec<HelpSectionSpec>, Vec<&'static str>) {
    use ContextHelpPreset::*;
    let navigation = "context-help-section-navigation";
    let actions = "context-help-section-actions";
    let editing = "context-help-section-editing";
    let workflow = "context-help-section-workflow";
    let search = "context-help-section-search";
    let selection = "context-help-section-selection";
    let tips = vec!["context-help-tip-ctrl-h"];
    let specs = match preset {
        Sessions => Some((
            "context-help-summary-sessions",
            vec![
                (
                    navigation,
                    vec![
                        ("Tab / Shift+Tab", "context-help-key-focus-next"),
                        ("j / k  ·  ↑ / ↓", "context-help-key-move"),
                        ("PageUp / PageDown", "context-help-key-page"),
                        ("Home / End", "context-help-key-first-last"),
                        ("Enter", "context-help-key-open"),
                    ],
                ),
                (
                    workflow,
                    vec![
                        ("1 / 2 / 3", "context-help-key-session-scope"),
                        ("m", "context-help-key-session-cycle"),
                    ],
                ),
            ],
            tips,
        )),
        Transcript => Some((
            "context-help-summary-transcript",
            vec![
                (
                    navigation,
                    vec![
                        ("Tab / Shift+Tab", "context-help-key-focus-next"),
                        ("j / k  ·  ↑ / ↓", "context-help-key-transcript-vertical"),
                        ("h / l  ·  ← / →", "context-help-key-editor-move"),
                        (
                            "0 / ^ / $  ·  w / b / e / ge",
                            "context-help-key-editor-move",
                        ),
                        ("f / F / t / T  ·  ; / ,", "context-help-key-editor-move"),
                        ("Ctrl+H / Ctrl+L", "context-help-key-transcript-horizontal"),
                        ("[count] + motion", "context-help-key-count"),
                        ("Space / Shift+Space / Ctrl+B", "context-help-key-page"),
                        ("PageUp / PageDown", "context-help-key-page"),
                        ("Ctrl+U / Ctrl+D", "context-help-key-half-page"),
                        (
                            "gg / G  ·  H / M / L  ·  zt / zz / zb",
                            "context-help-key-first-last",
                        ),
                    ],
                ),
                (
                    actions,
                    vec![
                        ("i", "context-help-key-insert-mode"),
                        ("Enter", "context-help-key-toggle"),
                        (
                            "v / V / Ctrl+V  ·  y / Esc",
                            "context-help-key-visual-select",
                        ),
                        ("o / O / gv  ·  vam / vaM", "context-help-key-visual-select"),
                        ("yy / Y / y{motion}  ·  yam / yaM", "context-help-key-copy"),
                        ("/  ?  n  N", "context-help-key-search-transcript"),
                    ],
                ),
            ],
            tips,
        )),
        Composer => Some((
            "context-help-summary-composer",
            vec![
                (
                    workflow,
                    vec![
                        ("Enter", "context-help-key-send"),
                        ("Ctrl+Enter", "context-help-key-send-now"),
                        ("Ctrl+J / Shift+Enter", "context-help-key-newline"),
                        ("Esc", "context-help-key-view-mode"),
                    ],
                ),
                (
                    editing,
                    vec![
                        ("↑ at start", "context-help-key-history"),
                                                ("Ctrl+P", "context-help-key-recover"),
                        ("Ctrl+X", "context-help-key-cancel-pending"),
                        ("Ctrl+C", "context-help-key-clear-composer"),
                        ("Ctrl+G", "context-help-key-items"),
                    ],
                ),
                (
                    actions,
                    vec![
                        ("/", "context-help-key-commands"),
                        ("Ctrl+A", "context-help-key-insert-content"),
                        ("Ctrl+O", "context-help-key-attach"),
                        ("Ctrl+E", "context-help-key-external-editor"),
                        ("Ctrl+T", "context-help-key-image"),
                        ("Ctrl+R / Ctrl+L", "context-help-key-pending-requests"),
                    ],
                ),
            ],
            tips,
        )),
        ComposerItems => Some((
            "context-help-summary-composer-items",
            vec![(
                navigation,
                vec![
                    ("Tab / → / l", "context-help-key-next"),
                    ("BackTab / ← / h", "context-help-key-previous"),
                    ("Enter / o", "context-help-key-open"),
                    ("Ctrl+D", "context-help-key-delete"),
                    ("Esc", "context-help-key-close"),
                ],
            )],
            tips,
        )),
        PromptHistory => Some((
            "context-help-summary-history",
            vec![(
                search,
                vec![
                    ("Type", "context-help-key-filter"),
                    ("← / → in search", "context-help-key-editor-move"),
                    ("↑", "context-help-key-previous"),
                    ("↓", "context-help-key-next"),
                    ("← / → in results", "context-help-key-page"),
                    ("Ctrl+R", "context-help-key-older"),
                    ("Ctrl+S", "context-help-key-newer-stay"),
                    ("Enter", "context-help-key-accept"),
                    ("Esc / Ctrl+C", "context-help-key-close"),
                ],
            )],
            tips,
        )),
        Suggestion => Some((
            "context-help-summary-suggestions",
            vec![(
                selection,
                vec![
                    ("Type", "context-help-key-filter"),
                    ("← / → in search", "context-help-key-editor-move"),
                    ("↑ / Ctrl+P", "context-help-key-previous"),
                    ("↓ / Ctrl+N", "context-help-key-next"),
                    ("← / → in results", "context-help-key-page"),
                    ("Tab", "context-help-key-fill"),
                    ("Enter", "context-help-key-accept"),
                    ("Esc", "context-help-key-close"),
                ],
            )],
            tips,
        )),
        SingleLineEditor => Some((
            "context-help-summary-editor",
            vec![(
                editing,
                vec![
                    ("Type", "context-help-key-edit-text"),
                    (
                        "← / →  ·  Ctrl+B/F  ·  Ctrl+Left/Right",
                        "context-help-key-editor-move",
                    ),
                    ("Home / End  ·  Ctrl+A/E", "context-help-key-editor-move"),
                    (
                        "Backspace  ·  Ctrl+D/W/U/K/Y",
                        "context-help-key-editor-edit",
                    ),
                    ("Enter", "context-help-key-submit"),
                    ("Esc", "context-help-key-cancel"),
                ],
            )],
            tips,
        )),
        MultiLineEditor => Some((
            "context-help-summary-editor-multiline",
            vec![(
                editing,
                vec![
                    ("Type", "context-help-key-edit-text"),
                    (
                        "Arrows  ·  Ctrl+B/F/P/N  ·  Ctrl+Left/Right",
                        "context-help-key-editor-move",
                    ),
                    ("Home / End  ·  Ctrl+A/E", "context-help-key-editor-move"),
                    (
                        "Backspace  ·  Ctrl+D/W/U/K/Y",
                        "context-help-key-editor-edit",
                    ),
                    ("Enter", "context-help-key-newline"),
                    ("Ctrl+S", "context-help-key-save"),
                    ("Esc", "context-help-key-cancel"),
                ],
            )],
            tips,
        )),
        SearchPicker => Some((
            "context-help-summary-search-picker",
            vec![(
                search,
                vec![
                    ("Type", "context-help-key-filter"),
                    ("← / → in search", "context-help-key-editor-move"),
                    ("↑ / ↓", "context-help-key-move"),
                    ("← / → in results", "context-help-key-page"),
                    ("Enter", "context-help-key-activate"),
                    ("Esc", "context-help-key-close"),
                ],
            )],
            tips,
        )),
        ChoiceList => Some((
            "context-help-summary-choice-list",
            vec![(
                selection,
                vec![
                    ("↑ / ↓", "context-help-key-move"),
                    ("← / →", "context-help-key-page"),
                    ("Enter", "context-help-key-accept"),
                    ("Esc", "context-help-key-close"),
                ],
            )],
            tips,
        )),
        Timeline => Some((
            "context-help-summary-timeline",
            vec![(
                search,
                vec![
                    ("Type", "context-help-key-filter"),
                    ("← / → in search", "context-help-key-editor-move"),
                    ("↑ / ↓", "context-help-key-move"),
                    ("← / → in results", "context-help-key-page"),
                    ("Enter", "context-help-key-jump-message"),
                    ("Esc", "context-help-key-close"),
                ],
            )],
            tips,
        )),
        Permission => Some((
            "context-help-summary-permission",
            vec![(
                selection,
                vec![
                    ("↑ / ↓", "context-help-key-move"),
                    ("Enter", "context-help-key-activate"),
                    ("Esc", "context-help-key-back"),
                ],
            )],
            tips,
        )),
        ReadOnlyDetails => Some((
            "context-help-summary-details",
            vec![(actions, vec![("Esc", "context-help-key-back")])],
            tips,
        )),
        UserInputQuestion => Some((
            "context-help-summary-user-input",
            vec![(
                selection,
                vec![
                    ("↑ / ↓", "context-help-key-move"),
                    ("PageUp / PageDown", "context-help-key-page"),
                    ("Space", "context-help-key-toggle"),
                    ("Tab / Shift+Tab", "context-help-key-next-question"),
                    ("e", "context-help-key-custom-answer"),
                    ("Ctrl+D", "context-help-key-clear"),
                    ("Enter", "context-help-key-submit"),
                    ("Ctrl+X / Esc", "context-help-key-cancel-request"),
                ],
            )],
            tips,
        )),
        UserInputEditor => Some((
            "context-help-summary-user-input-editor",
            vec![(
                editing,
                vec![
                    ("Type", "context-help-key-edit-text"),
                    ("Enter", "context-help-key-accept"),
                    ("Esc", "context-help-key-back"),
                    ("Ctrl+X", "context-help-key-cancel-request"),
                ],
            )],
            tips,
        )),
        UserInputReview => Some((
            "context-help-summary-user-input-review",
            vec![(
                selection,
                vec![
                    ("Tab / Shift+Tab", "context-help-key-next-question"),
                    ("↑ / ↓", "context-help-key-move"),
                    ("e", "context-help-key-edit-answer"),
                    ("Ctrl+D", "context-help-key-clear"),
                    ("Enter", "context-help-key-submit-all"),
                    ("Ctrl+X / Esc", "context-help-key-cancel-request"),
                ],
            )],
            tips,
        )),
        UserInputDecisionReview => Some((
            "context-help-summary-user-input-decision-review",
            vec![
                (
                    selection,
                    vec![
                        ("↑ / ↓", "context-help-key-move"),
                        ("Enter", "context-help-key-submit"),
                        ("Ctrl+X / Esc", "context-help-key-cancel-request"),
                    ],
                ),
                (
                    navigation,
                    vec![("PageUp / PageDown", "context-help-key-scroll-body")],
                ),
            ],
            tips,
        )),
        Confirm => Some((
            "context-help-summary-confirm",
            vec![(
                actions,
                vec![
                    ("Y / Enter", "context-help-key-confirm"),
                    ("Esc", "context-help-key-cancel"),
                ],
            )],
            tips,
        )),
        Usage => Some((
            "context-help-summary-usage",
            vec![
                (
                    actions,
                    vec![
                        ("Ctrl+P", "context-help-key-usage-period"),
                        ("Ctrl+B", "context-help-key-usage-view"),
                        ("Ctrl+O", "context-help-key-usage-provider"),
                        ("Ctrl+L", "context-help-key-usage-model"),
                        ("Ctrl+A", "context-help-key-usage-subagents"),
                        ("Ctrl+S", "context-help-key-usage-sort"),
                        ("Ctrl+R", "context-help-key-refresh"),
                    ],
                ),
                (
                    navigation,
                    vec![
                        ("↑ / ↓", "context-help-key-move"),
                        ("Enter", "context-help-key-open"),
                        ("Esc", "context-help-key-close"),
                    ],
                ),
            ],
            tips,
        )),
        BasicList => Some((
            "context-help-summary-list",
            vec![(
                navigation,
                vec![
                    ("↑ / ↓", "context-help-key-move"),
                    ("Enter", "context-help-key-activate"),
                    ("Esc", "context-help-key-back"),
                ],
            )],
            tips,
        )),
        Settings => Some((
            "context-help-summary-panes",
            vec![
                (actions, vec![("Ctrl+R", "context-help-key-refresh")]),
                (
                    navigation,
                    vec![
                        ("← / →", "context-help-key-pane-horizontal"),
                        ("Tab / Shift+Tab", "context-help-key-focus-next"),
                        ("↑ / ↓", "context-help-key-move"),
                        ("Enter", "context-help-key-activate"),
                        ("Esc", "context-help-key-back"),
                    ],
                ),
            ],
            tips,
        )),
        ActionPane => Some((
            "context-help-summary-action-pane",
            vec![(
                navigation,
                vec![
                    ("← / →", "context-help-key-pane-horizontal"),
                    ("Tab / Shift+Tab", "context-help-key-focus-next"),
                    ("↑ / ↓", "context-help-key-move"),
                    ("Ctrl+N", "context-help-key-permission-add"),
                    ("Enter", "context-help-key-activate"),
                    ("Ctrl+E", "context-help-key-permission-rename"),
                    ("Ctrl+D", "context-help-key-delete"),
                    ("Esc", "context-help-key-back"),
                ],
            )],
            tips,
        )),
        PermissionRule => Some((
            "context-help-summary-list",
            vec![(
                navigation,
                vec![
                    ("↑ / ↓", "context-help-key-move"),
                    ("Enter", "context-help-key-activate"),
                    ("Ctrl+O", "context-help-key-permission-browse"),
                    ("Ctrl+S", "context-help-key-save"),
                    ("Ctrl+D", "context-help-key-delete"),
                    ("Esc", "context-help-key-back"),
                ],
            )],
            tips,
        )),
        Provider => Some((
            "context-help-summary-provider",
            vec![
                (
                    actions,
                    vec![
                        ("Ctrl+R", "context-help-key-refresh"),
                        ("Ctrl+N", "context-help-key-provider-add-model"),
                        ("Ctrl+A", "context-help-key-provider-save-adapter"),
                        ("Ctrl+S", "context-help-key-save"),
                    ],
                ),
                (
                    navigation,
                    vec![
                        ("Tab / Shift+Tab", "context-help-key-focus-next"),
                        ("↑ / ↓", "context-help-key-move"),
                        ("Space", "context-help-key-toggle"),
                        ("Enter", "context-help-key-activate"),
                        ("Ctrl+D", "context-help-key-delete"),
                        ("Esc", "context-help-key-close"),
                    ],
                ),
            ],
            tips,
        )),
        ProviderModel => Some((
            "context-help-summary-list",
            vec![(
                navigation,
                vec![
                    ("↑ / ↓", "context-help-key-move"),
                    ("Enter", "context-help-key-activate"),
                    ("Ctrl+S", "context-help-key-save"),
                    ("Ctrl+D", "context-help-key-delete"),
                    ("Esc", "context-help-key-back"),
                ],
            )],
            tips,
        )),
        ModelCatalog => Some((
            "context-help-summary-model-catalog",
            vec![
                (
                    actions,
                    vec![
                        ("Ctrl+F", "context-help-key-model-catalog-search"),
                        ("Ctrl+R", "context-help-key-refresh"),
                        ("← / →", "context-help-key-model-catalog-page"),
                    ],
                ),
                (
                    navigation,
                    vec![
                        ("↑ / ↓", "context-help-key-move"),
                        ("Esc", "context-help-key-close"),
                    ],
                ),
            ],
            tips,
        )),
        PluginList => Some((
            "context-help-summary-plugin-list",
            vec![(
                search,
                vec![
                    ("Type", "context-help-key-filter"),
                    ("Ctrl+T", "context-help-key-plugin-transport"),
                    ("Ctrl+G", "context-help-key-plugin-config-filter"),
                    ("Ctrl+R", "context-help-key-refresh"),
                    ("↑ / ↓", "context-help-key-move"),
                    ("Enter", "context-help-key-open"),
                    ("Esc", "context-help-key-close"),
                ],
            )],
            tips,
        )),
        PluginDetail => Some((
            "context-help-summary-plugin-detail",
            vec![(
                navigation,
                vec![
                    ("Tab / Shift+Tab", "context-help-key-next-tab"),
                    ("↑ / ↓", "context-help-key-scroll"),
                    ("Esc", "context-help-key-back"),
                ],
            )],
            tips,
        )),
        PluginConfig => Some((
            "context-help-summary-plugin-config",
            vec![
                (
                    actions,
                    vec![
                        ("Ctrl+K", "context-help-key-plugin-validate"),
                        ("Ctrl+U", "context-help-key-plugin-reset"),
                        ("Ctrl+P", "context-help-key-plugin-diff"),
                        ("Ctrl+S", "context-help-key-save"),
                        ("Ctrl+R", "context-help-key-plugin-restart"),
                    ],
                ),
                (
                    navigation,
                    vec![
                        ("Tab / Shift+Tab", "context-help-key-focus-next"),
                        ("↑ / ↓", "context-help-key-move"),
                        ("← / →", "context-help-key-horizontal"),
                        ("Enter", "context-help-key-activate"),
                        ("Ctrl+D", "context-help-key-delete"),
                        ("Esc", "context-help-key-back"),
                    ],
                ),
            ],
            tips,
        )),
        PluginActions => Some((
            "context-help-summary-plugin-actions",
            vec![(
                actions,
                vec![
                    ("↑ / ↓", "context-help-key-move"),
                    ("← / →", "context-help-key-page"),
                    ("Enter", "context-help-key-activate"),
                    ("Esc", "context-help-key-close"),
                ],
            )],
            tips,
        )),
        PluginSelection => Some((
            "context-help-summary-plugin-selection",
            vec![(
                selection,
                vec![
                    ("↑ / ↓", "context-help-key-move"),
                    ("← / →", "context-help-key-page"),
                    ("Space", "context-help-key-toggle"),
                    ("Enter", "context-help-key-accept"),
                    ("Esc", "context-help-key-close"),
                ],
            )],
            tips,
        )),
        PluginDrilldown => Some((
            "context-help-summary-plugin-drilldown",
            vec![(
                navigation,
                vec![
                    ("↑ / ↓", "context-help-key-move"),
                    ("← / →", "context-help-key-horizontal"),
                    ("Enter", "context-help-key-activate"),
                    ("Ctrl+D", "context-help-key-delete"),
                    ("Esc", "context-help-key-back"),
                ],
            )],
            tips,
        )),
        PluginDiff => Some((
            "context-help-summary-plugin-diff",
            vec![(actions, vec![("Esc", "context-help-key-close")])],
            tips,
        )),
    };
    specs.expect("every ContextHelpPreset owns a TUI key card")
}

/// Distinguishes the two presentation uses of the shared help dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HelpOverlayKind {
    ContextHelp,
    Diagnostics,
}

/// TUI-owned presentation state for contextual help and diagnostics.
pub type HelpOverlay = HelpDialogState<HelpOverlayKind>;

/// Builds the canonical localized contextual-help document from a caller's
/// selected context and TUI-owned key-card specs. Route/overlay selection stays
/// with the application, while all help-dialog document construction stays in
/// the presentation crate.
pub fn contextual_document(
    i18n: &I18n,
    context: String,
    summary_key: &'static str,
    section_specs: Vec<HelpSectionSpec>,
    tip_keys: Vec<&'static str>,
) -> HelpOverlay {
    let sections = section_specs
        .into_iter()
        .map(|(title_key, entries)| HelpDialogSection {
            title: i18n.text(title_key),
            entries: entries
                .into_iter()
                .map(|(keys, description_key)| HelpDialogEntry {
                    keys: keys.to_owned(),
                    description: i18n.text(description_key),
                })
                .collect(),
        })
        .collect();
    HelpOverlay {
        kind: HelpOverlayKind::ContextHelp,
        modal_title: i18n.text("context-help-title"),
        eyebrow: i18n.text("context-help-eyebrow"),
        footer: i18n.text("context-help-footer"),
        context,
        summary: i18n.text(summary_key),
        sections,
        tips: tip_keys.into_iter().map(|key| i18n.text(key)).collect(),
        scroll: ScrollState::default(),
        max_scroll: 0,
    }
}

/// Builds the plain-text representation used by a caller-owned clipboard
/// effect. No App, route, terminal, or runtime state crosses this boundary.
pub fn plain_text(overlay: &HelpOverlay) -> String {
    let mut lines = vec![
        overlay.modal_title.clone(),
        overlay.context.clone(),
        overlay.summary.clone(),
    ];
    for section in &overlay.sections {
        lines.push(String::new());
        lines.push(format!("[{}]", section.title));
        lines.extend(
            section
                .entries
                .iter()
                .map(|entry| format!("{}: {}", entry.keys, entry.description)),
        );
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use agena_tui_components::{HelpDialogEntry, HelpDialogSection, ScrollState};

    use super::{ContextHelpPreset, HelpOverlay, HelpOverlayKind, contextual_document, plain_text};
    use crate::i18n::I18n;

    #[test]
    fn plain_text_retains_the_context_summary_and_entries() {
        let overlay = HelpOverlay {
            kind: HelpOverlayKind::Diagnostics,
            modal_title: "Diagnostics".to_owned(),
            eyebrow: "Terminal".to_owned(),
            footer: "Esc closes".to_owned(),
            context: "Kitty".to_owned(),
            summary: "Ready".to_owned(),
            sections: vec![HelpDialogSection {
                title: "Protocols".to_owned(),
                entries: vec![HelpDialogEntry {
                    keys: "Mouse".to_owned(),
                    description: "enabled".to_owned(),
                }],
            }],
            tips: Vec::new(),
            scroll: ScrollState::default(),
            max_scroll: 0,
        };

        assert_eq!(
            plain_text(&overlay),
            "Diagnostics\nKitty\nReady\n\n[Protocols]\nMouse: enabled"
        );
    }

    #[test]
    fn contextual_document_localizes_the_complete_key_card() {
        let overlay = contextual_document(
            &I18n::english(),
            "Composer".to_owned(),
            "context-help-summary-composer",
            vec![(
                "context-help-section-actions",
                vec![("Enter", "context-help-key-send")],
            )],
            vec!["context-help-tip-ctrl-h"],
        );

        assert_eq!(overlay.kind, HelpOverlayKind::ContextHelp);
        assert_eq!(overlay.context, "Composer");
        assert_eq!(overlay.sections.len(), 1);
        assert!(!overlay.summary.is_empty());
        assert!(!overlay.sections[0].title.is_empty());
        assert!(!overlay.sections[0].entries[0].description.is_empty());
    }

    #[test]
    fn contextual_presets_are_distinct_presentation_cards() {
        assert_ne!(
            ContextHelpPreset::Composer,
            ContextHelpPreset::ComposerItems
        );
        assert_ne!(
            ContextHelpPreset::PluginList,
            ContextHelpPreset::PluginDetail
        );
    }
}
