impl App {
    pub(crate) fn open_context_help(&mut self) {
        self.context_help = Some(self.build_context_help());
    }

    pub(crate) fn toggle_context_help(&mut self) {
        if self.context_help.is_some() {
            self.context_help = None;
        } else {
            self.open_context_help();
        }
    }

    pub(crate) fn handle_context_help_key(&mut self, key: KeyEvent) -> bool {
        let Some(help) = self.context_help.as_mut() else {
            return false;
        };
        if help.kind == HelpOverlayKind::Diagnostics
            && key.modifiers.is_empty()
            && matches!(key.code, KeyCode::Char('c' | 'y'))
        {
            let report = agena_tui::help::plain_text(help);
            self.request_clipboard_copy(
                report,
                ui_text::t(&self.i18n, "terminal-diagnostics-copied"),
            );
            return true;
        }
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
        if self.current_route_is_main() && self.composer_item_selection.is_active() {
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
                self.help_for(HelpPreset::Settings, dialog.title.clone())
            }
            Route::PermissionStudio(dialog) => {
                if let Some(editor) = dialog.editor.as_ref() {
                    self.help_for_editor(editor.title.clone(), editor.multiline)
                } else {
                    self.help_for(HelpPreset::ActionPane, dialog.title.clone())
                }
            }
            Route::PermissionRuleStudio(dialog) => {
                if let Some(editor) = dialog.editor.as_ref() {
                    self.help_for_editor(editor.title.clone(), editor.multiline)
                } else {
                    self.help_for(
                        HelpPreset::PermissionRule,
                        dialog.presentation.title.clone(),
                    )
                }
            }
            Route::SessionSearch(dialog) => {
                self.help_for(HelpPreset::SearchPicker, dialog.title.clone())
            }
            Route::CommandPalette(dialog) => {
                self.help_for(HelpPreset::SearchPicker, dialog.presentation.title.clone())
            }
            Route::SkillPicker(dialog) => {
                self.help_for(HelpPreset::SearchPicker, dialog.presentation.title.clone())
            }
            Route::SkillStudio(dialog) => {
                if let Some(editor) = dialog.editor.as_ref() {
                    self.help_for_editor(editor.title.clone(), editor.multiline)
                } else if dialog.detail.is_some() {
                    self.help_for(
                        HelpPreset::ReadOnlyDetails,
                        ui_text::t(&self.i18n, "overlay-skill-studio-title"),
                    )
                } else {
                    self.help_for(HelpPreset::SearchPicker, dialog.presentation.title.clone())
                }
            }
            Route::SessionNavigation(dialog) => {
                self.help_for(HelpPreset::SearchPicker, dialog.presentation.title.clone())
            }
            Route::SelectionPicker(dialog) => {
                self.help_for(HelpPreset::SearchPicker, dialog.presentation.title.clone())
            }
            Route::SessionModelChooser(dialog) => {
                self.help_for(HelpPreset::SearchPicker, dialog.title.clone())
            }
            Route::Timeline(dialog) => self.help_for(HelpPreset::Timeline, dialog.title.clone()),
            Route::PluginWorkbench(dialog) => self.help_for_plugin_workbench(dialog),
            Route::ProviderStudio(dialog) => self.help_for_provider(dialog),
            Route::ModelCatalogStudio(dialog) => self.help_for_model_catalog(dialog),
        }
    }

    fn help_for_overlay(&self, overlay: &Overlay) -> HelpOverlay {
        match overlay {
            Overlay::TranscriptSearch(dialog) | Overlay::SessionRename(dialog) => {
                self.help_for_editor(dialog.title.clone(), false)
            }
            Overlay::SettingsValueEdit(dialog) => self.help_for_editor(dialog.title.clone(), false),
            Overlay::Choice(dialog) => self.help_for(
                if dialog.presentation.config.input_mode.is_visible() {
                    HelpPreset::SearchPicker
                } else {
                    HelpPreset::ChoiceList
                },
                dialog.presentation.title.clone(),
            ),
            Overlay::PathBrowser(dialog) => {
                self.help_for(HelpPreset::SearchPicker, dialog.presentation.title.clone())
            }
            Overlay::Permission(dialog) => self.help_for(
                if dialog.presentation.page().is_details() {
                    HelpPreset::ReadOnlyDetails
                } else {
                    HelpPreset::Permission
                },
                if dialog.presentation.page().is_details() {
                    ui_text::t(&self.i18n, "overlay-permission-details-title")
                } else {
                    ui_text::t(&self.i18n, "overlay-permission-title")
                },
            ),
            Overlay::UserInputReply(dialog) => {
                let preset = if dialog.presentation.is_editing_custom() {
                    HelpPreset::UserInputEditor
                } else if Self::user_input_overlay_is_review(dialog) {
                    HelpPreset::UserInputDecisionReview
                } else if dialog.presentation.screen() == QuestionFlowScreen::Review {
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
                self.help_for(HelpPreset::SearchPicker, dialog.title.clone())
            }
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
            return self.help_for(HelpPreset::ProviderModel, page.title.clone());
        }
        if let Some(page) = dialog.detail_page.as_ref() {
            return self.help_for(HelpPreset::BasicList, page.title.clone());
        }
        self.help_for(HelpPreset::Provider, dialog.title.clone())
    }

    fn help_for_model_catalog(&self, dialog: &ModelCatalogStudioOverlay) -> HelpOverlay {
        if let Some(editor) = dialog.editor.as_ref() {
            self.help_for_editor(editor.title.clone(), false)
        } else {
            self.help_for(HelpPreset::ModelCatalog, dialog.presentation.title.clone())
        }
    }

    fn help_for_plugin_workbench(&self, dialog: &PluginWorkbenchOverlay) -> HelpOverlay {
        if let Some(editor) = dialog.tool_editor.as_ref() {
            return self.help_for_editor(editor.title.clone(), true);
        }
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
        match dialog.navigation.mode {
            PluginWorkbenchMode::List => self.help_for(
                HelpPreset::PluginList,
                ui_text::t(&self.i18n, "context-help-context-plugin-list"),
            ),
            PluginWorkbenchMode::Detail
                if dialog.navigation.detail_tab == PluginDetailTab::Config =>
            {
                self.help_for(
                    HelpPreset::PluginConfig,
                    ui_text::t(&self.i18n, "context-help-context-plugin-config"),
                )
            }
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
        let (summary_key, section_specs, tip_keys) = agena_tui::help::preset_specs(preset);
        agena_tui::help::contextual_document(
            &self.i18n,
            context,
            summary_key,
            section_specs,
            tip_keys,
        )
    }
}

impl App {
    pub(crate) fn open_terminal_diagnostics(&mut self) {
        let Some(context) = self.launch.terminal_context.as_ref() else {
            self.flash_warning(ui_text::t(&self.i18n, "terminal-diagnostics-unavailable"));
            return;
        };
        let identity = &context.identity;
        let text = |key| ui_text::t(&self.i18n, key);
        let configured_color = match self.launch.tui_config.color_scheme {
            agena_tui::presentation_config::ColorSchemePreference::Auto => {
                text("terminal-diagnostics-color-mode-auto")
            }
            agena_tui::presentation_config::ColorSchemePreference::Dark => {
                text("terminal-diagnostics-color-mode-dark")
            }
            agena_tui::presentation_config::ColorSchemePreference::Light => {
                text("terminal-diagnostics-color-mode-light")
            }
        };
        let detected_background = context
            .color
            .background
            .map(terminal_rgb_description)
            .unwrap_or_else(|| text("terminal-diagnostics-unknown"));
        let detected_appearance = context.color.background.map_or_else(
            || text("terminal-diagnostics-color-appearance-unknown"),
            |background| {
                text(if background.is_light() {
                    "terminal-diagnostics-color-appearance-light"
                } else {
                    "terminal-diagnostics-color-appearance-dark"
                })
            },
        );
        let effective_appearance = match self.launch.tui_config.color_scheme {
            agena_tui::presentation_config::ColorSchemePreference::Dark => {
                text("terminal-diagnostics-color-appearance-dark")
            }
            agena_tui::presentation_config::ColorSchemePreference::Light => {
                text("terminal-diagnostics-color-appearance-light")
            }
            agena_tui::presentation_config::ColorSchemePreference::Auto => {
                context.color.background.map_or_else(
                    || text("terminal-diagnostics-color-appearance-conservative"),
                    |background| {
                        text(if background.is_light() {
                            "terminal-diagnostics-color-appearance-light"
                        } else {
                            "terminal-diagnostics-color-appearance-dark"
                        })
                    },
                )
            }
        };
        let effective_background = self
            .launch
            .tui_config
            .graphics_background(self.launch.terminal_background);
        let formula_foreground = terminal_rgb_description(
            agena_tui_media::formula_foreground_for_background(effective_background),
        );
        let color_refresh = text(if context.color.source.supports_live_refresh() {
            "terminal-diagnostics-color-refresh-live"
        } else {
            "terminal-diagnostics-color-refresh-startup-only"
        });
        let mouse_event_count = if self.mouse_events_seen == 0 {
            text("terminal-diagnostics-mouse-events-none")
        } else {
            self.i18n.text_args(
                "terminal-diagnostics-mouse-events-seen",
                &agena_tui::fl_args!("count" => self.mouse_events_seen as i64),
            )
        };
        let last_mouse_event = self
            .last_mouse_event
            .clone()
            .unwrap_or_else(|| text("terminal-diagnostics-mouse-last-none"));
        let evidence = if identity.evidence.is_empty() {
            text("terminal-diagnostics-none")
        } else {
            identity
                .evidence
                .iter()
                .map(|item| {
                    let value = if matches!(
                        item.key,
                        "WT_SESSION"
                            | "KITTY_WINDOW_ID"
                            | "KITTY_PID"
                            | "WEZTERM_PANE"
                            | "ALACRITTY_SOCKET"
                            | "KONSOLE_DBUS_SERVICE"
                    ) {
                        "<present>"
                    } else {
                        item.value.as_str()
                    };
                    format!(
                        "{}={} → {} ({})",
                        item.key,
                        value,
                        item.candidate,
                        text(item.source.localization_key())
                    )
                })
                .collect::<Vec<_>>()
                .join("; ")
        };
        let conflicts = identity.conflicts();
        let conflicts = if conflicts.is_empty() {
            text("terminal-diagnostics-none")
        } else {
            conflicts
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        };
        let layers = if context.transport_evidence.is_empty() {
            vec![HelpEntry {
                keys: text("terminal-diagnostics-direct"),
                description: text("terminal-diagnostics-direct-description"),
            }]
        } else {
            context
                .transport_evidence
                .iter()
                .map(|item| HelpEntry {
                    keys: item.layer.label().to_owned(),
                    description: self.i18n.text_args(
                        "terminal-diagnostics-layer-description",
                        &agena_tui::fl_args!("source" => item.source_key),
                    ),
                })
                .collect()
        };
        let capability =
            |name: &str, value: agena_tui::terminal_capabilities::CapabilityEvidence| {
                let path = match value.path {
                    agena_tui::terminal_capabilities::CapabilityPath::Clear => {
                        "terminal-diagnostics-path-clear"
                    }
                    agena_tui::terminal_capabilities::CapabilityPath::UserForced => {
                        "terminal-diagnostics-path-forced"
                    }
                    agena_tui::terminal_capabilities::CapabilityPath::Unverified => {
                        "terminal-diagnostics-path-unverified"
                    }
                    agena_tui::terminal_capabilities::CapabilityPath::Blocked => {
                        "terminal-diagnostics-path-blocked"
                    }
                };
                let provider = match value.provider {
                    agena_tui::terminal_capabilities::ProviderReadiness::NotRequired => {
                        "terminal-diagnostics-provider-not-required"
                    }
                    agena_tui::terminal_capabilities::ProviderReadiness::Ready => {
                        "terminal-diagnostics-provider-ready"
                    }
                    agena_tui::terminal_capabilities::ProviderReadiness::Missing => {
                        "terminal-diagnostics-provider-missing"
                    }
                };
                HelpEntry {
                    keys: name.to_owned(),
                    description: self.i18n.text_args(
                        "terminal-diagnostics-capability-description",
                        &agena_tui::fl_args!(
                            "status" => text(value.support.localization_key()),
                            "source" => text(value.source.localization_key()),
                            "path" => text(path),
                            "provider" => text(provider),
                        ),
                    ),
                }
            };
        let capabilities = &context.capabilities;
        let mut provider_entries = Vec::new();
        if identity.family == agena_tui_platform::terminal::TerminalFamily::Kitty
            && let Some(helper) = agena_tui_platform::kitty::helper()
        {
            provider_entries.push(HelpEntry {
                keys: "Kitty helper".to_owned(),
                description: format!(
                    "{} · version={} · clipboard={} · transfer={}",
                    diagnostic_path(helper.path.as_path()),
                    helper.version.as_deref().unwrap_or("unknown"),
                    helper.clipboard,
                    helper.transfer,
                ),
            });
        } else if identity.family == agena_tui_platform::terminal::TerminalFamily::Kitty {
            provider_entries.push(HelpEntry {
                keys: "Kitty helper".to_owned(),
                description: text("terminal-diagnostics-helper-missing"),
            });
        } else {
            provider_entries.push(HelpEntry {
                keys: "Kitty helper".to_owned(),
                description: text("terminal-diagnostics-helper-not-probed"),
            });
        }
        for (label, path) in [
            (
                "iTerm2 upload",
                agena_tui_platform::iterm2::upload_utility(),
            ),
            (
                "iTerm2 download",
                agena_tui_platform::iterm2::download_utility(),
            ),
        ] {
            provider_entries.push(HelpEntry {
                keys: label.to_owned(),
                description: path
                    .map(|path| diagnostic_path(path.as_path()))
                    .unwrap_or_else(|| text("terminal-diagnostics-helper-missing")),
            });
        }
        let warning_entries = if context.diagnostics().is_empty() {
            vec![HelpEntry {
                keys: "✓".to_owned(),
                description: text("terminal-diagnostics-no-warnings"),
            }]
        } else {
            context
                .diagnostics()
                .iter()
                .map(|diagnostic| HelpEntry {
                    keys: diagnostic.code.to_owned(),
                    description: diagnostic.message.clone(),
                })
                .collect()
        };
        self.context_help = Some(HelpOverlay {
            kind: HelpOverlayKind::Diagnostics,
            modal_title: ui_text::t(&self.i18n, "terminal-diagnostics-title"),
            eyebrow: ui_text::t(&self.i18n, "terminal-diagnostics-eyebrow"),
            footer: ui_text::t(&self.i18n, "terminal-diagnostics-footer"),
            context: identity.display_name(),
            summary: self.i18n.text_args(
                "terminal-diagnostics-summary",
                &agena_tui::fl_args!("confidence" => text(identity.confidence.localization_key())),
            ),
            sections: vec![
                HelpSection {
                    title: text("terminal-diagnostics-section-identity"),
                    entries: vec![
                        HelpEntry {
                            keys: text("terminal-diagnostics-field-product"),
                            description: identity.family.to_string(),
                        },
                        HelpEntry {
                            keys: text("terminal-diagnostics-field-version"),
                            description: identity
                                .version
                                .clone()
                                .unwrap_or_else(|| text("terminal-diagnostics-unknown")),
                        },
                        HelpEntry {
                            keys: text("terminal-diagnostics-field-parsed-version"),
                            description: identity
                                .parsed_version
                                .as_ref()
                                .map(ToString::to_string)
                                .unwrap_or_else(|| text("terminal-diagnostics-unavailable-value")),
                        },
                        HelpEntry {
                            keys: text("terminal-diagnostics-field-compatibility"),
                            description: identity
                                .term
                                .clone()
                                .unwrap_or_else(|| text("terminal-diagnostics-term-unset")),
                        },
                        HelpEntry {
                            keys: text("terminal-diagnostics-field-confidence"),
                            description: text(identity.confidence.localization_key()),
                        },
                        HelpEntry {
                            keys: text("terminal-diagnostics-field-source"),
                            description: text(identity.source.localization_key()),
                        },
                        HelpEntry {
                            keys: text("terminal-diagnostics-field-evidence"),
                            description: evidence,
                        },
                        HelpEntry {
                            keys: text("terminal-diagnostics-field-conflicts"),
                            description: conflicts,
                        },
                    ],
                },
                HelpSection {
                    title: text("terminal-diagnostics-section-layers"),
                    entries: layers,
                },
                HelpSection {
                    title: text("terminal-diagnostics-section-color"),
                    entries: vec![
                        HelpEntry {
                            keys: text("terminal-diagnostics-color-configured"),
                            description: configured_color,
                        },
                        HelpEntry {
                            keys: text("terminal-diagnostics-color-detected-background"),
                            description: detected_background,
                        },
                        HelpEntry {
                            keys: text("terminal-diagnostics-color-detected-appearance"),
                            description: detected_appearance,
                        },
                        HelpEntry {
                            keys: text("terminal-diagnostics-color-source"),
                            description: text(context.color.source.localization_key()),
                        },
                        HelpEntry {
                            keys: text("terminal-diagnostics-color-refresh"),
                            description: color_refresh,
                        },
                        HelpEntry {
                            keys: text("terminal-diagnostics-color-generation"),
                            description: context.color_generation.to_string(),
                        },
                        HelpEntry {
                            keys: text("terminal-diagnostics-color-effective-appearance"),
                            description: effective_appearance,
                        },
                        HelpEntry {
                            keys: text("terminal-diagnostics-color-formula-foreground"),
                            description: formula_foreground,
                        },
                        HelpEntry {
                            keys: text("terminal-diagnostics-color-formula-background"),
                            description: text(
                                "terminal-diagnostics-color-formula-background-transparent",
                            ),
                        },
                        HelpEntry {
                            keys: text("terminal-diagnostics-color-background-images"),
                            description: text(
                                "terminal-diagnostics-color-background-images-not-sampled",
                            ),
                        },
                    ],
                },
                HelpSection {
                    title: text("terminal-diagnostics-section-protocols"),
                    entries: vec![
                        capability(
                            &text("terminal-diagnostics-protocol-alternate-screen"),
                            capabilities.alternate_screen,
                        ),
                        capability(
                            &text("terminal-diagnostics-protocol-bracketed-paste"),
                            capabilities.bracketed_paste,
                        ),
                        capability(
                            &text("terminal-diagnostics-protocol-focus"),
                            capabilities.focus_reporting,
                        ),
                        capability(
                            &text("terminal-diagnostics-protocol-mouse"),
                            capabilities.mouse_capture,
                        ),
                        HelpEntry {
                            keys: text("terminal-diagnostics-protocol-mouse-mode"),
                            description: text("terminal-diagnostics-mouse-mode-button-sgr"),
                        },
                        HelpEntry {
                            keys: text("terminal-diagnostics-protocol-mouse-events"),
                            description: mouse_event_count,
                        },
                        HelpEntry {
                            keys: text("terminal-diagnostics-protocol-mouse-last"),
                            description: last_mouse_event,
                        },
                        capability(
                            &text("terminal-diagnostics-protocol-keyboard"),
                            capabilities.keyboard_disambiguation,
                        ),
                        capability(
                            &text("terminal-diagnostics-protocol-key-events"),
                            capabilities.keyboard_event_types,
                        ),
                        capability(
                            &text("terminal-diagnostics-protocol-background"),
                            capabilities.default_color_query,
                        ),
                        capability(
                            &text("terminal-diagnostics-protocol-native-clipboard"),
                            capabilities.clipboard_write_native,
                        ),
                        capability(
                            &text("terminal-diagnostics-protocol-osc52-write"),
                            capabilities.clipboard_write_osc52,
                        ),
                        capability(
                            &text("terminal-diagnostics-protocol-osc52-read"),
                            capabilities.clipboard_read_osc52,
                        ),
                    ],
                },
                HelpSection {
                    title: text("terminal-diagnostics-section-providers"),
                    entries: {
                        let mut entries = vec![
                            capability(
                                &text("terminal-diagnostics-provider-kitty-clipboard"),
                                capabilities.kitty_rich_clipboard,
                            ),
                            capability(
                                &text("terminal-diagnostics-provider-kitty-transfer"),
                                capabilities.kitty_file_transfer,
                            ),
                            capability(
                                &text("terminal-diagnostics-provider-iterm-transfer"),
                                capabilities.iterm2_file_transfer,
                            ),
                            capability(
                                &text("terminal-diagnostics-provider-inline-images"),
                                capabilities.inline_images,
                            ),
                            capability(
                                &text("terminal-diagnostics-provider-hyperlinks"),
                                capabilities.hyperlinks,
                            ),
                            capability(
                                &text("terminal-diagnostics-provider-sync-output"),
                                capabilities.synchronized_output,
                            ),
                        ];
                        entries.extend(provider_entries);
                        entries
                    },
                },
                HelpSection {
                    title: text("terminal-diagnostics-section-warnings"),
                    entries: warning_entries,
                },
            ],
            tips: vec![ui_text::t(&self.i18n, "terminal-diagnostics-tip")],
            scroll: ScrollState::default(),
            max_scroll: 0,
        });
    }
}

fn diagnostic_path(path: &std::path::Path) -> String {
    if let Some(home) = std::env::var_os("HOME") {
        let home = std::path::Path::new(&home);
        if let Ok(relative) = path.strip_prefix(home) {
            return format!("~/{}", relative.display());
        }
    }
    path.display().to_string()
}

fn terminal_rgb_description(color: agena_tui_components::TerminalRgb) -> String {
    format!(
        "#{:02X}{:02X}{:02X} · rgb({}, {}, {})",
        color.red, color.green, color.blue, color.red, color.green, color.blue
    )
}

use crate::{
    App, HelpEntry, HelpOverlay, HelpOverlayKind, HelpSection, KeyEvent, ModelCatalogStudioOverlay,
    Overlay, PluginWorkbenchOverlay, ProviderStudioOverlay, Route, ui_text,
};
use agena_tui::help::ContextHelpPreset as HelpPreset;
use agena_tui::keymap::{KeyAction, KeyContext, resolve as resolve_tui_key};
use agena_tui::main_focus::Focus;
use agena_tui_components::QuestionFlowScreen;
use agena_tui_components::ScrollState;
use agena_tui_plugin_workbench::{PluginDetailTab, PluginWorkbenchMode};
use crossterm::event::KeyCode;

#[cfg(test)]
mod tests {
    use super::HelpPreset;
    use agena_tui::i18n::I18n;

    fn tui_preset_specs(
        preset: HelpPreset,
    ) -> (
        &'static str,
        Vec<agena_tui::help::HelpSectionSpec>,
        Vec<&'static str>,
    ) {
        agena_tui::help::preset_specs(preset)
    }

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
            HelpPreset::SearchPicker,
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
            HelpPreset::Settings,
            HelpPreset::ActionPane,
            HelpPreset::PermissionRule,
            HelpPreset::Provider,
            HelpPreset::ProviderModel,
            HelpPreset::ModelCatalog,
            HelpPreset::PluginList,
            HelpPreset::PluginDetail,
            HelpPreset::PluginConfig,
            HelpPreset::PluginActions,
            HelpPreset::PluginSelection,
            HelpPreset::PluginDrilldown,
            HelpPreset::PluginDiff,
        ];

        for preset in presets {
            let (summary, sections, tips) = tui_preset_specs(preset);
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
            HelpPreset::BasicList,
            HelpPreset::Settings,
            HelpPreset::ActionPane,
            HelpPreset::PermissionRule,
            HelpPreset::Provider,
            HelpPreset::ProviderModel,
            HelpPreset::ModelCatalog,
            HelpPreset::PluginList,
            HelpPreset::PluginDetail,
            HelpPreset::PluginConfig,
        ] {
            let (_, sections, _) = tui_preset_specs(preset);
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
    fn settings_help_advertises_both_directional_and_tab_pane_navigation() {
        let (_, sections, _) = tui_preset_specs(HelpPreset::Settings);
        let keys = sections
            .iter()
            .flat_map(|(_, entries)| entries)
            .map(|(keys, _)| *keys)
            .collect::<Vec<_>>();

        assert!(keys.contains(&"← / →"));
        assert!(keys.contains(&"Tab / Shift+Tab"));
        assert!(keys.contains(&"Ctrl+R"));
    }

    #[test]
    fn deletable_selections_advertise_only_the_shared_delete_key() {
        for preset in [
            HelpPreset::ComposerItems,
            HelpPreset::ActionPane,
            HelpPreset::PermissionRule,
            HelpPreset::Provider,
            HelpPreset::ProviderModel,
            HelpPreset::PluginConfig,
            HelpPreset::PluginDrilldown,
        ] {
            let (_, sections, _) = tui_preset_specs(preset);
            let delete_keys = sections
                .iter()
                .flat_map(|(_, entries)| entries)
                .filter_map(|(keys, description)| {
                    (*description == "context-help-key-delete").then_some(*keys)
                })
                .collect::<Vec<_>>();

            assert_eq!(
                delete_keys,
                vec!["Ctrl+D"],
                "{preset:?} must advertise exactly one portable delete shortcut",
            );
        }
    }

    #[test]
    fn provider_help_advertises_all_page_level_actions() {
        let (_, sections, _) = tui_preset_specs(HelpPreset::Provider);
        let keys = sections
            .iter()
            .flat_map(|(_, entries)| entries)
            .map(|(keys, _)| *keys)
            .collect::<Vec<_>>();

        for expected in ["Ctrl+R", "Ctrl+N", "Ctrl+A", "Ctrl+S", "Ctrl+D"] {
            assert!(
                keys.contains(&expected),
                "missing Provider shortcut {expected}"
            );
        }
    }

    #[test]
    fn permission_help_uses_the_shared_delete_shortcut() {
        let (_, sections, _) = tui_preset_specs(HelpPreset::ActionPane);
        let delete_keys = sections
            .iter()
            .flat_map(|(_, entries)| entries)
            .filter_map(|(keys, description)| {
                (*description == "context-help-key-delete").then_some(*keys)
            })
            .collect::<Vec<_>>();

        assert_eq!(delete_keys, vec!["Ctrl+D"]);
    }

    #[test]
    fn plugin_restart_uses_a_portable_control_shortcut() {
        let (_, sections, _) = tui_preset_specs(HelpPreset::PluginConfig);
        let restart_keys = sections
            .iter()
            .flat_map(|(_, entries)| entries)
            .filter_map(|(keys, description)| {
                (*description == "context-help-key-plugin-restart").then_some(*keys)
            })
            .collect::<Vec<_>>();

        assert_eq!(restart_keys, vec!["Ctrl+R"]);
    }

    #[test]
    fn secondary_surface_help_avoids_alt_function_and_delete_keys() {
        for preset in [
            HelpPreset::Composer,
            HelpPreset::ComposerItems,
            HelpPreset::PromptHistory,
            HelpPreset::SingleLineEditor,
            HelpPreset::MultiLineEditor,
            HelpPreset::UserInputQuestion,
            HelpPreset::UserInputReview,
            HelpPreset::Usage,
            HelpPreset::Settings,
            HelpPreset::ActionPane,
            HelpPreset::PermissionRule,
            HelpPreset::Provider,
            HelpPreset::ProviderModel,
            HelpPreset::ModelCatalog,
            HelpPreset::PluginList,
            HelpPreset::PluginDetail,
            HelpPreset::PluginConfig,
            HelpPreset::PluginActions,
            HelpPreset::PluginSelection,
            HelpPreset::PluginDrilldown,
        ] {
            let (_, sections, _) = tui_preset_specs(preset);
            for (keys, _) in sections.iter().flat_map(|(_, entries)| entries) {
                assert!(
                    !keys.contains("Alt+"),
                    "{preset:?} advertises Alt/Option: {keys}"
                );
                assert!(
                    !keys.contains("Delete"),
                    "{preset:?} requires Delete: {keys}"
                );
                assert!(
                    !keys.split([' ', '/', '·']).any(|key| matches!(
                        key,
                        "F1" | "F2"
                            | "F3"
                            | "F4"
                            | "F5"
                            | "F6"
                            | "F7"
                            | "F8"
                            | "F9"
                            | "F10"
                            | "F11"
                            | "F12"
                    )),
                    "{preset:?} requires a function key: {keys}",
                );
            }
        }
    }

    #[test]
    fn every_paginated_search_picker_advertises_horizontal_page_navigation() {
        for preset in [
            HelpPreset::PromptHistory,
            HelpPreset::Suggestion,
            HelpPreset::SearchPicker,
            HelpPreset::ChoiceList,
            HelpPreset::Timeline,
            HelpPreset::PluginActions,
            HelpPreset::PluginSelection,
        ] {
            let (_, sections, _) = tui_preset_specs(preset);
            assert!(
                sections
                    .iter()
                    .flat_map(|(_, entries)| entries)
                    .any(|(keys, _)| keys.contains("← / →")),
                "{preset:?} must expose the shared horizontal page navigation"
            );
            assert!(
                sections
                    .iter()
                    .flat_map(|(_, entries)| entries)
                    .all(|(keys, _)| !keys.contains("PageUp")),
                "{preset:?} must not advertise legacy page keys"
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
            HelpPreset::SearchPicker,
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
            HelpPreset::Settings,
            HelpPreset::ActionPane,
            HelpPreset::PermissionRule,
            HelpPreset::Provider,
            HelpPreset::ProviderModel,
            HelpPreset::ModelCatalog,
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
        keys.extend([
            "terminal-diagnostics-title",
            "terminal-diagnostics-eyebrow",
            "terminal-diagnostics-footer",
            "terminal-diagnostics-tip",
            "terminal-diagnostics-copied",
            "terminal-diagnostics-unavailable",
            "terminal-diagnostics-summary",
            "terminal-diagnostics-none",
            "terminal-diagnostics-unknown",
            "terminal-diagnostics-unavailable-value",
            "terminal-diagnostics-term-unset",
            "terminal-diagnostics-section-identity",
            "terminal-diagnostics-section-layers",
            "terminal-diagnostics-section-color",
            "terminal-diagnostics-section-protocols",
            "terminal-diagnostics-section-providers",
            "terminal-diagnostics-section-warnings",
            "terminal-diagnostics-field-product",
            "terminal-diagnostics-field-version",
            "terminal-diagnostics-field-parsed-version",
            "terminal-diagnostics-field-compatibility",
            "terminal-diagnostics-field-confidence",
            "terminal-diagnostics-field-source",
            "terminal-diagnostics-field-evidence",
            "terminal-diagnostics-field-conflicts",
            "terminal-diagnostics-color-configured",
            "terminal-diagnostics-color-detected-background",
            "terminal-diagnostics-color-detected-appearance",
            "terminal-diagnostics-color-source",
            "terminal-diagnostics-color-refresh",
            "terminal-diagnostics-color-generation",
            "terminal-diagnostics-color-effective-appearance",
            "terminal-diagnostics-color-formula-foreground",
            "terminal-diagnostics-color-formula-background",
            "terminal-diagnostics-color-background-images",
            "terminal-diagnostics-color-mode-auto",
            "terminal-diagnostics-color-mode-dark",
            "terminal-diagnostics-color-mode-light",
            "terminal-diagnostics-color-appearance-dark",
            "terminal-diagnostics-color-appearance-light",
            "terminal-diagnostics-color-appearance-unknown",
            "terminal-diagnostics-color-appearance-conservative",
            "terminal-diagnostics-color-source-osc11",
            "terminal-diagnostics-color-source-iterm-osc4",
            "terminal-diagnostics-color-source-colorfgbg",
            "terminal-diagnostics-color-source-term-background",
            "terminal-diagnostics-color-source-vscode-theme",
            "terminal-diagnostics-color-source-unavailable",
            "terminal-diagnostics-color-refresh-live",
            "terminal-diagnostics-color-refresh-startup-only",
            "terminal-diagnostics-color-formula-background-transparent",
            "terminal-diagnostics-color-background-images-not-sampled",
            "terminal-diagnostics-direct",
            "terminal-diagnostics-direct-description",
            "terminal-diagnostics-layer-description",
            "terminal-diagnostics-capability-description",
            "terminal-diagnostics-path-clear",
            "terminal-diagnostics-path-forced",
            "terminal-diagnostics-path-unverified",
            "terminal-diagnostics-path-blocked",
            "terminal-diagnostics-provider-not-required",
            "terminal-diagnostics-provider-ready",
            "terminal-diagnostics-provider-missing",
            "terminal-diagnostics-helper-missing",
            "terminal-diagnostics-helper-not-probed",
            "terminal-diagnostics-no-warnings",
            "terminal-diagnostics-protocol-alternate-screen",
            "terminal-diagnostics-protocol-bracketed-paste",
            "terminal-diagnostics-protocol-focus",
            "terminal-diagnostics-protocol-mouse",
            "terminal-diagnostics-protocol-mouse-mode",
            "terminal-diagnostics-protocol-mouse-events",
            "terminal-diagnostics-protocol-mouse-last",
            "terminal-diagnostics-mouse-mode-button-sgr",
            "terminal-diagnostics-mouse-events-none",
            "terminal-diagnostics-mouse-events-seen",
            "terminal-diagnostics-mouse-last-none",
            "terminal-diagnostics-protocol-keyboard",
            "terminal-diagnostics-protocol-key-events",
            "terminal-diagnostics-protocol-background",
            "terminal-diagnostics-protocol-native-clipboard",
            "terminal-diagnostics-protocol-osc52-write",
            "terminal-diagnostics-protocol-osc52-read",
            "terminal-diagnostics-provider-kitty-clipboard",
            "terminal-diagnostics-provider-kitty-transfer",
            "terminal-diagnostics-provider-iterm-transfer",
            "terminal-diagnostics-provider-inline-images",
            "terminal-diagnostics-provider-hyperlinks",
            "terminal-diagnostics-provider-sync-output",
            "terminal-diagnostics-status-confirmed",
            "terminal-diagnostics-status-forced",
            "terminal-diagnostics-status-profiled",
            "terminal-diagnostics-status-unsupported",
            "terminal-diagnostics-status-unknown",
            "terminal-diagnostics-source-user",
            "terminal-diagnostics-source-environment",
            "terminal-diagnostics-source-helper",
            "terminal-diagnostics-source-profile",
            "terminal-diagnostics-source-platform",
            "terminal-diagnostics-source-conservative",
            "terminal-diagnostics-source-terminfo",
            "terminal-diagnostics-source-unknown",
            "terminal-diagnostics-confidence-explicit",
            "terminal-diagnostics-confidence-strong",
            "terminal-diagnostics-confidence-compatibility",
            "terminal-diagnostics-confidence-unknown",
        ]);
        for preset in presets {
            let (summary, sections, tips) = tui_preset_specs(preset);
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
