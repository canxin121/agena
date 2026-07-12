#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HelpPreset {
    Sessions,
    Transcript,
    Composer,
    ComposerItems,
    PromptHistory,
    Suggestion,
    SingleLineEditor,
    MultiLineEditor,
    SearchList,
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
    PaneList,
    ActionPane,
    Provider,
    PluginPolicy,
    PluginList,
    PluginDetail,
    PluginConfig,
    PluginActions,
    PluginSelection,
    PluginDrilldown,
    PluginDiff,
}

impl App {
    pub(in crate::app) fn open_context_help(&mut self) {
        self.context_help = Some(self.build_context_help());
    }

    pub(in crate::app) fn toggle_context_help(&mut self) {
        if self.context_help.is_some() {
            self.context_help = None;
        } else {
            self.open_context_help();
        }
    }

    pub(in crate::app) fn handle_context_help_key(&mut self, key: KeyEvent) -> bool {
        let Some(help) = self.context_help.as_mut() else {
            return false;
        };
        match resolve_tui_key(KeyContext::Help, key) {
            Some(KeyAction::Close) => self.context_help = None,
            Some(KeyAction::MoveUp) => help.scroll.move_by(-1, help.max_scroll),
            Some(KeyAction::MoveDown) => help.scroll.move_by(1, help.max_scroll),
            _ => {}
        }
        true
    }

    fn build_context_help(&self) -> HelpOverlay {
        if let Some(overlay) = self.overlay.as_ref() {
            return self.help_for_overlay(overlay);
        }
        if self.prompt_history_search.is_some() {
            return self.help_for(
                HelpPreset::PromptHistory,
                ui_text::t(&self.i18n, "composer-prompt-history-title"),
            );
        }
        if self.current_route_is_main() && self.selected_composer_item.is_some() {
            return self.help_for(
                HelpPreset::ComposerItems,
                ui_text::t(&self.i18n, "context-help-context-composer-items"),
            );
        }
        if self.current_route_is_main()
            && (self.slash_command_suggestions.is_some() || self.file_mention_suggestions.is_some())
        {
            return self.help_for(
                HelpPreset::Suggestion,
                ui_text::t(&self.i18n, "context-help-context-suggestions"),
            );
        }
        match &self.current_route {
            Route::Main => match self.focus {
                Focus::Sessions => self.help_for(
                    HelpPreset::Sessions,
                    ui_text::t(&self.i18n, "help-section-sessions"),
                ),
                Focus::Transcript => self.help_for(
                    HelpPreset::Transcript,
                    ui_text::t(&self.i18n, "help-section-transcript"),
                ),
                Focus::Composer => self.help_for(
                    HelpPreset::Composer,
                    ui_text::t(&self.i18n, "help-section-composer"),
                ),
            },
            Route::Usage(_) => self.help_for(
                HelpPreset::Usage,
                ui_text::t(&self.i18n, "context-help-context-usage"),
            ),
            Route::SettingsStudio(dialog) => {
                self.help_for(HelpPreset::PaneList, dialog.title.clone())
            }
            Route::AgentStudio(dialog) => {
                if let Some(editor) = dialog.workbench.editor.as_ref() {
                    self.help_for_editor(editor.title.clone(), editor.multiline)
                } else {
                    self.help_for(HelpPreset::BasicList, dialog.workbench.title.clone())
                }
            }
            Route::PermissionStudio(dialog) => {
                if let Some(editor) = dialog.editor.as_ref() {
                    self.help_for_editor(editor.title.clone(), editor.multiline)
                } else {
                    self.help_for(HelpPreset::ActionPane, dialog.title.clone())
                }
            }
            Route::PermissionRuleStudio(dialog) => {
                if let Some(editor) = dialog.workbench.editor.as_ref() {
                    self.help_for_editor(editor.title.clone(), editor.multiline)
                } else {
                    self.help_for(HelpPreset::BasicList, dialog.workbench.title.clone())
                }
            }
            Route::SessionSearch(dialog) => {
                self.help_for(HelpPreset::SearchList, dialog.title.clone())
            }
            Route::Picker(dialog) => self.help_for(HelpPreset::SearchList, dialog.title.clone()),
            Route::SessionModelChooser(dialog) => {
                self.help_for(HelpPreset::SearchList, dialog.title.clone())
            }
            Route::Timeline(dialog) => self.help_for(HelpPreset::Timeline, dialog.title.clone()),
            Route::PluginPolicyStudio(dialog) => {
                self.help_for(HelpPreset::PluginPolicy, dialog.title.clone())
            }
            Route::PluginWorkbench(dialog) => self.help_for_plugin_workbench(dialog),
            Route::ProviderStudio(dialog) => self.help_for_provider(dialog),
            Route::ModelCatalogStudio(dialog) => self.help_for_model_catalog(dialog),
        }
    }

    fn help_for_overlay(&self, overlay: &Overlay) -> HelpOverlay {
        match overlay {
            Overlay::TranscriptSearch(dialog)
            | Overlay::SessionRename(dialog)
            | Overlay::AgentCreate(dialog) => self.help_for_editor(dialog.title.clone(), false),
            Overlay::SettingsValueEdit(dialog) => self.help_for_editor(dialog.title.clone(), false),
            Overlay::RuntimeSettingEdit(dialog) => {
                self.help_for_editor(dialog.title.clone(), false)
            }
            Overlay::Choice(dialog) => self.help_for(
                if dialog.config.input_enabled {
                    HelpPreset::SearchList
                } else {
                    HelpPreset::ChoiceList
                },
                dialog.title.clone(),
            ),
            Overlay::FileAttach(dialog) => {
                self.help_for(HelpPreset::SearchList, dialog.title.clone())
            }
            Overlay::PathBrowser(dialog) => {
                self.help_for(HelpPreset::SearchList, dialog.title.clone())
            }
            Overlay::Permission(dialog) => self.help_for(
                if matches!(dialog.page, PermissionOverlayPage::Details(_)) {
                    HelpPreset::ReadOnlyDetails
                } else {
                    HelpPreset::Permission
                },
                if matches!(dialog.page, PermissionOverlayPage::Details(_)) {
                    ui_text::t(&self.i18n, "overlay-permission-details-title")
                } else {
                    ui_text::t(&self.i18n, "overlay-permission-title")
                },
            ),
            Overlay::UserInputReply(dialog) => {
                let preset = if dialog.editing_custom {
                    HelpPreset::UserInputEditor
                } else if Self::user_input_overlay_is_review(dialog) {
                    HelpPreset::UserInputDecisionReview
                } else if dialog.state.screen() == QuestionFlowScreen::Review {
                    HelpPreset::UserInputReview
                } else {
                    HelpPreset::UserInputQuestion
                };
                self.help_for(
                    preset,
                    ui_text::t(&self.i18n, "context-help-context-user-input"),
                )
            }
            Overlay::Confirm(dialog) => self.help_for(HelpPreset::Confirm, dialog.title.clone()),
            Overlay::SessionSearch(dialog) => {
                self.help_for(HelpPreset::SearchList, dialog.title.clone())
            }
            Overlay::Picker(dialog) => self.help_for(HelpPreset::SearchList, dialog.title.clone()),
            Overlay::Timeline(dialog) => self.help_for(HelpPreset::Timeline, dialog.title.clone()),
            Overlay::ProviderStudio(dialog) => self.help_for_provider(dialog),
            Overlay::ModelCatalogStudio(dialog) => self.help_for_model_catalog(dialog),
        }
    }

    fn help_for_provider(&self, dialog: &ProviderStudioOverlay) -> HelpOverlay {
        if let Some(editor) = dialog.editor.as_ref() {
            return self.help_for_editor(editor.title.clone(), editor.multiline);
        }
        if let Some(page) = dialog.model_page.as_ref() {
            return self.help_for(HelpPreset::BasicList, page.title.clone());
        }
        if let Some(page) = dialog.detail_page.as_ref() {
            return self.help_for(HelpPreset::BasicList, page.title.clone());
        }
        self.help_for(HelpPreset::Provider, dialog.title.clone())
    }

    fn help_for_model_catalog(&self, dialog: &ModelCatalogStudioOverlay) -> HelpOverlay {
        if let Some(editor) = dialog.workbench.editor.as_ref() {
            self.help_for_editor(editor.title.clone(), false)
        } else {
            self.help_for(HelpPreset::PaneList, dialog.workbench.title.clone())
        }
    }

    fn help_for_plugin_workbench(&self, dialog: &PluginWorkbenchOverlay) -> HelpOverlay {
        if let Some(editor) = dialog.editor.as_ref() {
            return self.help_for_editor(editor.title.clone(), editor.multiline);
        }
        if dialog.actions.is_some() {
            return self.help_for(
                HelpPreset::PluginActions,
                ui_text::t(&self.i18n, "context-help-context-plugin-actions"),
            );
        }
        if dialog.selection.is_some() {
            return self.help_for(
                HelpPreset::PluginSelection,
                ui_text::t(&self.i18n, "context-help-context-plugin-selection"),
            );
        }
        if !dialog.drilldown_stack.is_empty() {
            return self.help_for(
                HelpPreset::PluginDrilldown,
                ui_text::t(&self.i18n, "context-help-context-plugin-drilldown"),
            );
        }
        if dialog.show_diff {
            return self.help_for(
                HelpPreset::PluginDiff,
                ui_text::t(&self.i18n, "context-help-context-plugin-diff"),
            );
        }
        match dialog.mode {
            PluginWorkbenchMode::List => self.help_for(
                HelpPreset::PluginList,
                ui_text::t(&self.i18n, "context-help-context-plugin-list"),
            ),
            PluginWorkbenchMode::Detail if dialog.detail_tab == PluginDetailTab::Config => self
                .help_for(
                    HelpPreset::PluginConfig,
                    ui_text::t(&self.i18n, "context-help-context-plugin-config"),
                ),
            PluginWorkbenchMode::Detail => self.help_for(
                HelpPreset::PluginDetail,
                ui_text::t(&self.i18n, "context-help-context-plugin-detail"),
            ),
        }
    }

    fn help_for_editor(&self, title: String, multiline: bool) -> HelpOverlay {
        self.help_for(
            if multiline {
                HelpPreset::MultiLineEditor
            } else {
                HelpPreset::SingleLineEditor
            },
            title,
        )
    }

    fn help_for(&self, preset: HelpPreset, context: String) -> HelpOverlay {
        let (summary_key, section_specs, tip_keys) = help_preset(preset);
        let sections = section_specs
            .into_iter()
            .map(|(title_key, entries)| HelpSection {
                title: ui_text::t(&self.i18n, title_key),
                entries: entries
                    .into_iter()
                    .map(|(keys, description_key)| HelpEntry {
                        keys: keys.to_string(),
                        description: ui_text::t(&self.i18n, description_key),
                    })
                    .collect(),
            })
            .collect();
        HelpOverlay {
            context,
            summary: ui_text::t(&self.i18n, summary_key),
            sections,
            tips: tip_keys
                .into_iter()
                .map(|key| ui_text::t(&self.i18n, key))
                .collect(),
            scroll: ScrollState::default(),
            max_scroll: 0,
        }
    }
}

type HelpEntrySpec = (&'static str, &'static str);
type HelpSectionSpec = (&'static str, Vec<HelpEntrySpec>);

fn help_preset(preset: HelpPreset) -> (&'static str, Vec<HelpSectionSpec>, Vec<&'static str>) {
    use HelpPreset::*;
    let navigation = "context-help-section-navigation";
    let actions = "context-help-section-actions";
    let editing = "context-help-section-editing";
    let workflow = "context-help-section-workflow";
    let search = "context-help-section-search";
    let selection = "context-help-section-selection";
    let tips = vec!["context-help-tip-ctrl-h"];
    match preset {
        Sessions => (
            "context-help-summary-sessions",
            vec![
                (
                    navigation,
                    vec![
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
        ),
        Transcript => (
            "context-help-summary-transcript",
            vec![
                (
                    navigation,
                    vec![
                        ("j / k  ·  ↑ / ↓", "context-help-key-transcript-vertical"),
                        ("h / l  ·  ← / →", "context-help-key-transcript-horizontal"),
                        ("[count] + motion", "context-help-key-count"),
                        ("Space / Shift+Space / Ctrl+B", "context-help-key-page"),
                        ("PageUp / PageDown", "context-help-key-page"),
                        ("Ctrl+U / Ctrl+D", "context-help-key-half-page"),
                        ("g / G  ·  Home / End", "context-help-key-first-last"),
                    ],
                ),
                (
                    actions,
                    vec![
                        ("i", "context-help-key-insert-mode"),
                        ("Enter", "context-help-key-toggle"),
                        ("y / c / Y / C", "context-help-key-copy"),
                        ("/  ?  n  N", "context-help-key-search-transcript"),
                    ],
                ),
            ],
            tips,
        ),
        Composer => (
            "context-help-summary-composer",
            vec![
                (
                    workflow,
                    vec![
                        ("Enter", "context-help-key-send"),
                        ("Ctrl+Enter", "context-help-key-send-now"),
                        (
                            "Ctrl+J / Shift+Enter / Alt+Enter",
                            "context-help-key-newline",
                        ),
                        ("Esc", "context-help-key-view-mode"),
                    ],
                ),
                (
                    editing,
                    vec![
                        ("Ctrl+A/E/B/F/P/N", "context-help-key-editor-move"),
                        ("Ctrl+D/W/U/K/Y", "context-help-key-editor-edit"),
                        ("Ctrl+R / Alt+Up", "context-help-key-history"),
                        ("Ctrl+Up", "context-help-key-recover"),
                        ("Ctrl+L", "context-help-key-clear-composer"),
                        ("F2", "context-help-key-items"),
                    ],
                ),
                (
                    actions,
                    vec![
                        ("/", "context-help-key-commands"),
                        ("F3 / Ctrl+O / Alt+O", "context-help-key-attach"),
                        ("F4 / Alt+E", "context-help-key-external-editor"),
                        ("F6 / Alt+I", "context-help-key-image"),
                        ("Alt+U / Alt+A", "context-help-key-pending-requests"),
                    ],
                ),
            ],
            tips,
        ),
        ComposerItems => (
            "context-help-summary-composer-items",
            vec![(
                navigation,
                vec![
                    ("Tab / → / l", "context-help-key-next"),
                    ("BackTab / ← / h", "context-help-key-previous"),
                    ("Enter / o", "context-help-key-open"),
                    ("Delete / Backspace / d", "context-help-key-delete"),
                    ("Esc", "context-help-key-close"),
                ],
            )],
            tips,
        ),
        PromptHistory => (
            "context-help-summary-history",
            vec![(
                search,
                vec![
                    ("Type", "context-help-key-filter"),
                    ("↑ / Alt+Up / Ctrl+R", "context-help-key-older"),
                    ("↓ / Alt+Down", "context-help-key-newer"),
                    ("Ctrl+S", "context-help-key-newer-stay"),
                    ("Enter", "context-help-key-accept"),
                    ("Esc / Ctrl+C", "context-help-key-close"),
                ],
            )],
            tips,
        ),
        Suggestion => (
            "context-help-summary-suggestions",
            vec![(
                selection,
                vec![
                    ("↑ / Ctrl+P", "context-help-key-previous"),
                    ("↓ / Ctrl+N", "context-help-key-next"),
                    ("Tab", "context-help-key-fill"),
                    ("Enter", "context-help-key-accept"),
                    ("Esc", "context-help-key-close"),
                ],
            )],
            tips,
        ),
        SingleLineEditor => (
            "context-help-summary-editor",
            vec![(
                editing,
                vec![
                    ("Type", "context-help-key-edit-text"),
                    (
                        "← / →  ·  Ctrl+B/F  ·  Alt+B/F",
                        "context-help-key-editor-move",
                    ),
                    ("Home / End  ·  Ctrl+A/E", "context-help-key-editor-move"),
                    (
                        "Backspace / Delete  ·  Ctrl+D/W/U/K/Y",
                        "context-help-key-editor-edit",
                    ),
                    ("Enter", "context-help-key-submit"),
                    ("Esc", "context-help-key-cancel"),
                ],
            )],
            tips,
        ),
        MultiLineEditor => (
            "context-help-summary-editor-multiline",
            vec![(
                editing,
                vec![
                    ("Type", "context-help-key-edit-text"),
                    (
                        "Arrows  ·  Ctrl+B/F/P/N  ·  Alt+B/F",
                        "context-help-key-editor-move",
                    ),
                    ("Home / End  ·  Ctrl+A/E", "context-help-key-editor-move"),
                    (
                        "Backspace / Delete  ·  Ctrl+D/W/U/K/Y",
                        "context-help-key-editor-edit",
                    ),
                    ("Enter", "context-help-key-newline"),
                    ("Ctrl+S", "context-help-key-save"),
                    ("Esc", "context-help-key-cancel"),
                ],
            )],
            tips,
        ),
        SearchList => (
            "context-help-summary-search-list",
            vec![(
                search,
                vec![
                    ("Type", "context-help-key-filter"),
                    ("↑ / ↓", "context-help-key-move"),
                    ("Enter", "context-help-key-activate"),
                    ("Esc", "context-help-key-close"),
                ],
            )],
            tips,
        ),
        ChoiceList => (
            "context-help-summary-choice-list",
            vec![(
                selection,
                vec![
                    ("↑ / ↓", "context-help-key-move"),
                    ("Enter", "context-help-key-accept"),
                    ("Esc", "context-help-key-close"),
                ],
            )],
            tips,
        ),
        Timeline => (
            "context-help-summary-timeline",
            vec![(
                search,
                vec![
                    ("Type", "context-help-key-filter"),
                    ("↑ / ↓", "context-help-key-move"),
                    ("Enter", "context-help-key-jump-message"),
                    ("Esc", "context-help-key-close"),
                ],
            )],
            tips,
        ),
        Permission => (
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
        ),
        ReadOnlyDetails => (
            "context-help-summary-details",
            vec![(actions, vec![("Esc", "context-help-key-back")])],
            tips,
        ),
        UserInputQuestion => (
            "context-help-summary-user-input",
            vec![(
                selection,
                vec![
                    ("↑ / ↓", "context-help-key-move"),
                    ("Space", "context-help-key-toggle"),
                    ("Tab", "context-help-key-next-question"),
                    ("e", "context-help-key-custom-answer"),
                    ("Delete", "context-help-key-clear"),
                    ("Enter", "context-help-key-submit"),
                    ("Ctrl+X / Esc", "context-help-key-cancel-request"),
                ],
            )],
            tips,
        ),
        UserInputEditor => (
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
        ),
        UserInputReview => (
            "context-help-summary-user-input-review",
            vec![(
                selection,
                vec![
                    ("↑ / ↓", "context-help-key-move"),
                    ("e", "context-help-key-edit-answer"),
                    ("Delete", "context-help-key-clear"),
                    ("Enter", "context-help-key-submit-all"),
                    ("Ctrl+X / Esc", "context-help-key-cancel-request"),
                ],
            )],
            tips,
        ),
        UserInputDecisionReview => (
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
        ),
        Confirm => (
            "context-help-summary-confirm",
            vec![(
                actions,
                vec![
                    ("Enter", "context-help-key-confirm"),
                    ("Esc", "context-help-key-cancel"),
                ],
            )],
            tips,
        ),
        Usage => (
            "context-help-summary-usage",
            vec![(
                navigation,
                vec![
                    ("Tab", "context-help-key-focus-next"),
                    ("↑ / ↓", "context-help-key-move"),
                    ("Enter", "context-help-key-activate"),
                    ("Esc", "context-help-key-close"),
                ],
            )],
            tips,
        ),
        BasicList => (
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
        ),
        PaneList => (
            "context-help-summary-panes",
            vec![(
                navigation,
                vec![
                    ("Tab", "context-help-key-focus-next"),
                    ("↑ / ↓", "context-help-key-move"),
                    ("Enter", "context-help-key-activate"),
                    ("Esc", "context-help-key-back"),
                ],
            )],
            tips,
        ),
        ActionPane => (
            "context-help-summary-action-pane",
            vec![(
                navigation,
                vec![
                    ("Tab", "context-help-key-focus-next"),
                    ("↑ / ↓", "context-help-key-move"),
                    ("← / →", "context-help-key-horizontal"),
                    ("Enter", "context-help-key-activate"),
                    ("Esc", "context-help-key-back"),
                ],
            )],
            tips,
        ),
        Provider => (
            "context-help-summary-provider",
            vec![(
                navigation,
                vec![
                    ("Tab", "context-help-key-focus-next"),
                    ("↑ / ↓", "context-help-key-move"),
                    ("Space", "context-help-key-toggle"),
                    ("Enter", "context-help-key-activate"),
                    ("Esc", "context-help-key-close"),
                ],
            )],
            tips,
        ),
        PluginPolicy => (
            "context-help-summary-plugin-policy",
            vec![(
                navigation,
                vec![
                    ("Tab", "context-help-key-focus-next"),
                    ("↑ / ↓", "context-help-key-move"),
                    ("← / →", "context-help-key-horizontal"),
                    ("Enter", "context-help-key-cycle-value"),
                    ("Delete", "context-help-key-clear"),
                    ("Esc", "context-help-key-close"),
                ],
            )],
            tips,
        ),
        PluginList => (
            "context-help-summary-plugin-list",
            vec![(
                search,
                vec![
                    ("Type", "context-help-key-filter"),
                    ("Tab", "context-help-key-focus-next"),
                    ("↑ / ↓", "context-help-key-move"),
                    ("Enter", "context-help-key-activate"),
                    ("Esc", "context-help-key-close"),
                ],
            )],
            tips,
        ),
        PluginDetail => (
            "context-help-summary-plugin-detail",
            vec![(
                navigation,
                vec![
                    ("Tab", "context-help-key-next-tab"),
                    ("↑ / ↓", "context-help-key-scroll"),
                    ("Esc", "context-help-key-back"),
                ],
            )],
            tips,
        ),
        PluginConfig => (
            "context-help-summary-plugin-config",
            vec![(
                navigation,
                vec![
                    ("Tab", "context-help-key-focus-next"),
                    ("↑ / ↓", "context-help-key-move"),
                    ("← / →", "context-help-key-horizontal"),
                    ("Enter", "context-help-key-activate"),
                    ("Esc", "context-help-key-back"),
                ],
            )],
            tips,
        ),
        PluginActions => (
            "context-help-summary-plugin-actions",
            vec![(
                actions,
                vec![
                    ("↑ / ↓", "context-help-key-move"),
                    ("Enter", "context-help-key-activate"),
                    ("Esc", "context-help-key-close"),
                ],
            )],
            tips,
        ),
        PluginSelection => (
            "context-help-summary-plugin-selection",
            vec![(
                selection,
                vec![
                    ("↑ / ↓", "context-help-key-move"),
                    ("Space", "context-help-key-toggle"),
                    ("Enter", "context-help-key-accept"),
                    ("Esc", "context-help-key-close"),
                ],
            )],
            tips,
        ),
        PluginDrilldown => (
            "context-help-summary-plugin-drilldown",
            vec![(
                navigation,
                vec![
                    ("↑ / ↓", "context-help-key-move"),
                    ("← / →", "context-help-key-horizontal"),
                    ("Enter", "context-help-key-activate"),
                    ("Esc", "context-help-key-back"),
                ],
            )],
            tips,
        ),
        PluginDiff => (
            "context-help-summary-plugin-diff",
            vec![(actions, vec![("Esc", "context-help-key-close")])],
            tips,
        ),
    }
}

use crate::app::{
    App, Focus, HelpEntry, HelpOverlay, HelpSection, KeyEvent, ModelCatalogStudioOverlay, Overlay,
    PermissionOverlayPage, PluginDetailTab, PluginWorkbenchMode, PluginWorkbenchOverlay,
    ProviderStudioOverlay, QuestionFlowScreen, Route, ui_text,
};
use crate::tui_keymap::{KeyAction, KeyContext, resolve as resolve_tui_key};
use agena_tui_components::ScrollState;

#[cfg(test)]
mod tests {
    use super::{HelpPreset, help_preset};
    use crate::i18n::I18n;

    #[test]
    fn every_context_help_preset_has_focused_content() {
        let presets = [
            HelpPreset::Sessions,
            HelpPreset::Transcript,
            HelpPreset::Composer,
            HelpPreset::ComposerItems,
            HelpPreset::PromptHistory,
            HelpPreset::Suggestion,
            HelpPreset::SingleLineEditor,
            HelpPreset::MultiLineEditor,
            HelpPreset::SearchList,
            HelpPreset::ChoiceList,
            HelpPreset::Timeline,
            HelpPreset::Permission,
            HelpPreset::ReadOnlyDetails,
            HelpPreset::UserInputQuestion,
            HelpPreset::UserInputEditor,
            HelpPreset::UserInputReview,
            HelpPreset::UserInputDecisionReview,
            HelpPreset::Confirm,
            HelpPreset::Usage,
            HelpPreset::BasicList,
            HelpPreset::PaneList,
            HelpPreset::ActionPane,
            HelpPreset::Provider,
            HelpPreset::PluginPolicy,
            HelpPreset::PluginList,
            HelpPreset::PluginDetail,
            HelpPreset::PluginConfig,
            HelpPreset::PluginActions,
            HelpPreset::PluginSelection,
            HelpPreset::PluginDrilldown,
            HelpPreset::PluginDiff,
        ];

        for preset in presets {
            let (summary, sections, tips) = help_preset(preset);
            assert!(!summary.is_empty(), "{preset:?} needs an introduction");
            assert!(!sections.is_empty(), "{preset:?} needs its own key card");
            assert!(
                sections.iter().all(|(_, entries)| !entries.is_empty()),
                "{preset:?} contains an empty card"
            );
            assert!(!tips.is_empty(), "{preset:?} needs the global help hint");
        }
    }

    #[test]
    fn page_keys_only_appear_when_they_are_semantically_distinct() {
        for preset in [
            HelpPreset::Composer,
            HelpPreset::SearchList,
            HelpPreset::BasicList,
            HelpPreset::PaneList,
            HelpPreset::ActionPane,
            HelpPreset::Provider,
            HelpPreset::PluginList,
            HelpPreset::PluginDetail,
            HelpPreset::PluginConfig,
        ] {
            let (_, sections, _) = help_preset(preset);
            assert!(
                sections
                    .iter()
                    .flat_map(|(_, entries)| entries)
                    .all(|(keys, _)| !keys.contains("PageUp")),
                "{preset:?} reintroduced redundant page navigation"
            );
        }
    }

    #[test]
    fn contextual_help_strings_exist_in_english_and_chinese() {
        let locales = [I18n::english(), I18n::resolve(Some("zh-CN"), None)];
        let presets = [
            HelpPreset::Sessions,
            HelpPreset::Transcript,
            HelpPreset::Composer,
            HelpPreset::ComposerItems,
            HelpPreset::PromptHistory,
            HelpPreset::Suggestion,
            HelpPreset::SingleLineEditor,
            HelpPreset::MultiLineEditor,
            HelpPreset::SearchList,
            HelpPreset::ChoiceList,
            HelpPreset::Timeline,
            HelpPreset::Permission,
            HelpPreset::ReadOnlyDetails,
            HelpPreset::UserInputQuestion,
            HelpPreset::UserInputEditor,
            HelpPreset::UserInputReview,
            HelpPreset::UserInputDecisionReview,
            HelpPreset::Confirm,
            HelpPreset::Usage,
            HelpPreset::BasicList,
            HelpPreset::PaneList,
            HelpPreset::ActionPane,
            HelpPreset::Provider,
            HelpPreset::PluginPolicy,
            HelpPreset::PluginList,
            HelpPreset::PluginDetail,
            HelpPreset::PluginConfig,
            HelpPreset::PluginActions,
            HelpPreset::PluginSelection,
            HelpPreset::PluginDrilldown,
            HelpPreset::PluginDiff,
        ];
        let mut keys = vec![
            "context-help-title",
            "context-help-eyebrow",
            "context-help-footer",
            "context-help-global-hint",
            "context-help-context-composer-items",
            "context-help-context-suggestions",
            "context-help-context-usage",
            "context-help-context-user-input",
            "context-help-context-plugin-list",
            "context-help-context-plugin-detail",
            "context-help-context-plugin-config",
            "context-help-context-plugin-actions",
            "context-help-context-plugin-selection",
            "context-help-context-plugin-drilldown",
            "context-help-context-plugin-diff",
        ];
        for preset in presets {
            let (summary, sections, tips) = help_preset(preset);
            keys.push(summary);
            keys.extend(tips);
            for (section, entries) in sections {
                keys.push(section);
                keys.extend(entries.into_iter().map(|(_, description)| description));
            }
        }
        keys.sort_unstable();
        keys.dedup();

        for locale in locales {
            for key in &keys {
                assert_ne!(
                    locale.text(key),
                    *key,
                    "missing {key} for {}",
                    locale.locale_tag()
                );
            }
        }
    }
}
