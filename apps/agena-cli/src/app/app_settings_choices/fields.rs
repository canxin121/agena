impl App {
    pub(in crate::app) fn settings_field_choice_items(
        &self,
        field: SettingsFieldSpec,
    ) -> Option<Vec<ChoiceItem>> {
        match field.path {
            "providers.default" => {
                let fallback_adapter = settings_choice_adapter_fallback(&self.i18n);
                Some(
                    self.backend
                        .list_providers()
                        .into_iter()
                        .map(|provider| {
                            choice_item(
                                provider.provider_id,
                                settings_choice_default_provider_detail(
                                    &self.i18n,
                                    provider
                                        .defaults
                                        .adapter
                                        .as_deref()
                                        .unwrap_or(fallback_adapter.as_str()),
                                    provider.defaults.model.as_str(),
                                ),
                            )
                        })
                        .collect(),
                )
            }
            "agents.default" => Some(
                self.backend
                    .list_agent_names()
                    .into_iter()
                    .map(|agent| {
                        choice_item(agent, settings_choice_registered_agent_detail(&self.i18n))
                    })
                    .collect(),
            ),
            "ui.locale" => Some(
                SUPPORTED_LOCALES
                    .iter()
                    .map(|(code, detail)| choice_item(*code, *detail))
                    .collect(),
            ),
            "ui.tui.color_scheme" => Some(
                [
                    ("auto", "settings-choice-tui-color-scheme-auto"),
                    ("dark", "settings-choice-tui-color-scheme-dark"),
                    ("light", "settings-choice-tui-color-scheme-light"),
                ]
                .into_iter()
                .map(|(value, detail_key)| choice_item(value, ui_text::t(&self.i18n, detail_key)))
                .collect(),
            ),
            "ui.tui.theme" => Some(
                self.backend
                    .plugin_theme_palettes()
                    .into_iter()
                    .map(|theme| choice_item(theme.id, theme.display_name))
                    .collect(),
            ),
            "ui.tui.graphics" => Some(
                [
                    ("auto", "settings-choice-tui-graphics-auto"),
                    ("native", "settings-choice-tui-graphics-native"),
                    ("unicode", "settings-choice-tui-graphics-unicode"),
                ]
                .into_iter()
                .map(|(value, detail_key)| choice_item(value, ui_text::t(&self.i18n, detail_key)))
                .collect(),
            ),
            "tracing.filter" | "tracing.database" | "tracing.adapter" => Some(
                ["off", "error", "warn", "info", "debug", "trace"]
                    .into_iter()
                    .map(|level| choice_item(level, "log level"))
                    .collect(),
            ),
            _ if matches!(field.kind, SettingsFieldKind::Bool) => Some(boolean_choice_items(
                settings_choice_bool_override_detail(&self.i18n).as_str(),
            )),
            _ => None,
        }
    }

    pub(in crate::app) fn settings_field_choice_overlay_style(
        field: SettingsFieldSpec,
    ) -> ChoiceOverlayStyle {
        match field.path {
            "providers.default" | "agents.default" => ChoiceOverlayStyle::SearchableSelect,
            "ui.locale"
            | "ui.tui.color_scheme"
            | "ui.tui.graphics"
            | "tracing.filter"
            | "tracing.database"
            | "tracing.adapter" => ChoiceOverlayStyle::SelectOnly,
            "ui.tui.theme" => ChoiceOverlayStyle::SearchableSelect,
            _ if matches!(field.kind, SettingsFieldKind::Bool) => ChoiceOverlayStyle::SelectOnly,
            _ => ChoiceOverlayStyle::Searchable,
        }
    }

    pub(in crate::app) fn runtime_setting_choice_items(
        &mut self,
        field: RuntimeSettingSpec,
    ) -> Option<Vec<ChoiceItem>> {
        match field.id {
            RuntimeSettingId::ThinkingMode => match self
                .backend
                .runtime_thinking_mode_rows(&self.run_options.to_request())
            {
                Ok(rows) => Some(inspector_rows_to_mode_choice_items(
                    rows,
                    ui_text::thinking_mode_display_value,
                )),
                Err(error) => {
                    self.flash_warning(error.to_string());
                    Some(Vec::new())
                }
            },
            RuntimeSettingId::SpeedMode => match self
                .backend
                .runtime_speed_mode_rows(&self.run_options.to_request())
            {
                Ok(rows) => Some(inspector_rows_to_mode_choice_items(
                    rows,
                    ui_text::speed_mode_display_value,
                )),
                Err(error) => {
                    self.flash_warning(error.to_string());
                    Some(Vec::new())
                }
            },
            RuntimeSettingId::Verbosity => match self
                .backend
                .runtime_verbosity_values(&self.run_options.to_request())
            {
                Ok(values) => Some(
                    values
                        .into_iter()
                        .map(|value| {
                            choice_item(
                                value,
                                runtime_setting_choice_supported_model_detail(&self.i18n),
                            )
                        })
                        .collect(),
                ),
                Err(error) => {
                    self.flash_warning(error.to_string());
                    Some(Vec::new())
                }
            },
            RuntimeSettingId::ParallelToolCalls => Some(boolean_choice_items(
                runtime_setting_choice_parallel_detail(&self.i18n).as_str(),
            )),
            RuntimeSettingId::Temperature
            | RuntimeSettingId::MaxOutput
            | RuntimeSettingId::System => None,
        }
    }

    pub(in crate::app) fn runtime_setting_choice_overlay_style(
        field: RuntimeSettingSpec,
    ) -> ChoiceOverlayStyle {
        match field.id {
            RuntimeSettingId::ParallelToolCalls => ChoiceOverlayStyle::SelectOnly,
            RuntimeSettingId::ThinkingMode
            | RuntimeSettingId::SpeedMode
            | RuntimeSettingId::Verbosity
            | RuntimeSettingId::Temperature
            | RuntimeSettingId::MaxOutput
            | RuntimeSettingId::System => ChoiceOverlayStyle::Searchable,
        }
    }

    pub(in crate::app) fn provider_studio_field_choice_items(
        &self,
        dialog: &ProviderStudioOverlay,
        field: ProviderStudioField,
    ) -> Option<Vec<ChoiceItem>> {
        match field {
            ProviderStudioField::AuthMode => Some(vec![
                choice_item(
                    "none",
                    ui_text::t(&self.i18n, "provider-auth-mode-none-detail"),
                ),
                choice_item(
                    "api",
                    ui_text::t(&self.i18n, "provider-auth-mode-api-detail"),
                ),
                choice_item(
                    "credential",
                    ui_text::t(&self.i18n, "provider-auth-mode-credential-detail"),
                ),
            ]),
            ProviderStudioField::AuthSubtype => match dialog.draft.auth_kind {
                ProviderDraftAuthKind::ApiPending
                | ProviderDraftAuthKind::Api
                | ProviderDraftAuthKind::ClineApi
                | ProviderDraftAuthKind::Gitlab
                | ProviderDraftAuthKind::BedrockSigv4 => Some(vec![
                    choice_item(
                        "custom",
                        ui_text::t(&self.i18n, "provider-auth-subtype-custom-detail"),
                    ),
                    choice_item(
                        "cline_api",
                        ui_text::t(&self.i18n, "provider-auth-subtype-cline-api-detail"),
                    ),
                    choice_item(
                        "gitlab_api",
                        ui_text::t(&self.i18n, "provider-auth-subtype-gitlab-api-detail"),
                    ),
                    choice_item(
                        "bedrock_sigv4",
                        ui_text::t(&self.i18n, "provider-auth-subtype-bedrock-detail"),
                    ),
                ]),
                ProviderDraftAuthKind::Credential(_) => Some(vec![
                    choice_item(
                        "openai_chatgpt",
                        ui_text::t(&self.i18n, "provider-issuer-openai-chatgpt-detail"),
                    ),
                    choice_item(
                        "github_copilot",
                        ui_text::t(&self.i18n, "provider-issuer-github-copilot-detail"),
                    ),
                    choice_item(
                        "gitlab",
                        ui_text::t(&self.i18n, "provider-issuer-gitlab-detail"),
                    ),
                    choice_item(
                        "google_adc",
                        ui_text::t(&self.i18n, "provider-issuer-google-adc-detail"),
                    ),
                    choice_item(
                        "sap_ai_core",
                        ui_text::t(&self.i18n, "provider-issuer-sap-ai-core-detail"),
                    ),
                ]),
                ProviderDraftAuthKind::Unset | ProviderDraftAuthKind::None => None,
            },
            ProviderStudioField::AuthLoginMethod => {
                let items = match dialog.draft.auth_kind.credential_issuer() {
                    Some(CredentialIssuer::OpenaiChatgpt) => vec![
                        choice_item(
                            "device",
                            ui_text::t(&self.i18n, "provider-auth-login-kind-device-detail"),
                        ),
                        choice_item(
                            "browser",
                            ui_text::t(&self.i18n, "provider-auth-login-kind-browser-detail"),
                        ),
                    ],
                    Some(CredentialIssuer::GithubCopilot) => vec![choice_item(
                        "device",
                        ui_text::t(&self.i18n, "provider-auth-login-kind-device-detail"),
                    )],
                    Some(CredentialIssuer::Gitlab) => vec![choice_item(
                        "browser",
                        ui_text::t(&self.i18n, "provider-auth-login-kind-browser-detail"),
                    )],
                    Some(CredentialIssuer::GoogleAdc | CredentialIssuer::SapAiCore) | None => {
                        Vec::new()
                    }
                };
                (!items.is_empty()).then_some(items)
            }
            ProviderStudioField::InstanceUrl => Some(vec![choice_item(
                "https://gitlab.com",
                ui_text::t(&self.i18n, "provider-instance-url-gitlab-detail"),
            )]),
            ProviderStudioField::RedirectUri => Some(vec![choice_item(
                "http://localhost:1455/auth/callback",
                ui_text::t(&self.i18n, "provider-redirect-local-copy-detail"),
            )]),
            ProviderStudioField::Region => Some(
                AWS_REGION_CHOICES
                    .iter()
                    .map(|region| {
                        choice_item(
                            *region,
                            ui_text::t(&self.i18n, "provider-region-choice-detail"),
                        )
                    })
                    .collect(),
            ),
            ProviderStudioField::Profile => Some(provider_studio_profile_choice_items(
                &self.i18n,
                &self.backend,
            )),
            ProviderStudioField::ApiKeySource => Some(vec![
                choice_item(
                    "inline",
                    ui_text::t(&self.i18n, "provider-api-key-source-inline-detail"),
                ),
                choice_item(
                    "env",
                    ui_text::t(&self.i18n, "provider-api-key-source-env-detail"),
                ),
            ]),
            ProviderStudioField::ApiKeyValue
                if matches!(
                    dialog.draft.auth.secret_source_kind,
                    ProviderDraftSecretSourceKind::Env
                ) =>
            {
                Some(provider_studio_api_key_env_choice_items(&self.i18n))
            }
            ProviderStudioField::ApiKeyValue => None,
            ProviderStudioField::ServiceKeyEnv => Some(vec![choice_item(
                "AICORE_SERVICE_KEY",
                ui_text::t(&self.i18n, "provider-service-key-env-detail"),
            )]),
            ProviderStudioField::DefaultAdapter => Some(
                dialog
                    .adapter_candidate_ids
                    .iter()
                    .map(|adapter_id| {
                        let detail = provider_studio_adapter_rule(dialog, adapter_id.as_str())
                            .map(|rule| {
                                let mut parts =
                                    vec![provider_studio_adapter_rule_detail(&self.i18n, rule)];
                                if dialog.configured_adapter_ids.contains(adapter_id) {
                                    parts.push(ui_text::t(
                                        &self.i18n,
                                        "overlay-provider-studio-configured",
                                    ));
                                }
                                join_inline_segments(parts)
                            })
                            .unwrap_or_else(|| {
                                if dialog.configured_adapter_ids.contains(adapter_id) {
                                    ui_text::t(
                                        &self.i18n,
                                        "overlay-provider-studio-configured-disk",
                                    )
                                } else {
                                    ui_text::t(&self.i18n, "overlay-provider-studio-not-supported")
                                }
                            });
                        choice_item(adapter_id.clone(), detail)
                    })
                    .collect(),
            ),
            ProviderStudioField::DefaultModel => Some(provider_studio_default_model_choice_items(
                &self.i18n, dialog,
            )),
            _ => None,
        }
    }

    pub(in crate::app) fn provider_studio_field_choice_overlay_style(
        field: ProviderStudioField,
    ) -> ChoiceOverlayStyle {
        match field {
            ProviderStudioField::AuthMode
            | ProviderStudioField::AuthSubtype
            | ProviderStudioField::AuthLoginMethod
            | ProviderStudioField::InstanceUrl
            | ProviderStudioField::RedirectUri
            | ProviderStudioField::ApiKeySource
            | ProviderStudioField::ServiceKeyEnv => ChoiceOverlayStyle::SelectOnly,
            ProviderStudioField::Region
            | ProviderStudioField::Profile
            | ProviderStudioField::DefaultAdapter
            | ProviderStudioField::DefaultModel => ChoiceOverlayStyle::Searchable,
            _ => ChoiceOverlayStyle::Searchable,
        }
    }

    pub(in crate::app) fn provider_model_config_field_choice_items(
        &self,
        dialog: &ProviderStudioOverlay,
        field: ProviderModelConfigField,
    ) -> Option<Vec<ChoiceItem>> {
        match field {
            ProviderModelConfigField::Enabled => Some(boolean_choice_items(
                ui_text::t(&self.i18n, "provider-model-enabled-detail").as_str(),
            )),
            ProviderModelConfigField::AgenaToolTransport => Some(vec![
                choice_item_with_value(
                    ui_text::t(&self.i18n, "agena-tool-transport-provider-protocol-label"),
                    "provider_protocol",
                    ui_text::t(&self.i18n, "agena-tool-transport-provider-protocol-detail"),
                ),
                choice_item_with_value(
                    ui_text::t(&self.i18n, "agena-tool-transport-prompt-envelope-label"),
                    "prompt_envelope",
                    ui_text::t(&self.i18n, "agena-tool-transport-prompt-envelope-detail"),
                ),
            ]),
            ProviderModelConfigField::Lifecycle => Some(
                [
                    "active",
                    "preview",
                    "beta",
                    "alpha",
                    "experimental",
                    "deprecated",
                ]
                .into_iter()
                .map(|value| {
                    choice_item(
                        value,
                        ui_text::t(&self.i18n, "provider-model-lifecycle-detail"),
                    )
                })
                .collect(),
            ),
            ProviderModelConfigField::ProviderTools => {
                let mut items = vec![choice_item(
                    ProviderToolsPreset::Disabled.token(),
                    ui_text::t(&self.i18n, "provider-tools-disabled-detail"),
                )];
                if let Some(adapter_id) = dialog
                    .model_page
                    .as_ref()
                    .map(|page| page.adapter_id.as_str())
                    && let Some(preset) = provider_tools_available_preset_for_adapter(adapter_id)
                {
                    let detail_key = match preset {
                        ProviderToolsPreset::OpenAiHostedDefaults => "provider-tools-openai-detail",
                        ProviderToolsPreset::AnthropicHostedDefaults => {
                            "provider-tools-anthropic-detail"
                        }
                        ProviderToolsPreset::GeminiHostedDefaults => "provider-tools-gemini-detail",
                        ProviderToolsPreset::Disabled | ProviderToolsPreset::Custom => {
                            unreachable!()
                        }
                    };
                    items.push(choice_item(
                        preset.token(),
                        ui_text::t(&self.i18n, detail_key),
                    ));
                }
                if dialog.model_page.as_ref().is_some_and(|page| {
                    page.draft.provider_tools_preset == ProviderToolsPreset::Custom
                }) {
                    items.push(choice_item(
                        ProviderToolsPreset::Custom.token(),
                        ui_text::t(&self.i18n, "provider-tools-custom-detail"),
                    ));
                }
                Some(items)
            }
            ProviderModelConfigField::ModelId
            | ProviderModelConfigField::DisplayName
            | ProviderModelConfigField::ContextWindowTokens
            | ProviderModelConfigField::MaxInputTokens
            | ProviderModelConfigField::MaxOutputTokens
            | ProviderModelConfigField::InputModalities
            | ProviderModelConfigField::Features
            | ProviderModelConfigField::OutputModalities
            | ProviderModelConfigField::ThinkingModeVariants
            | ProviderModelConfigField::SpeedModeVariants
            | ProviderModelConfigField::Description => None,
        }
    }

    pub(in crate::app) fn provider_model_config_field_choice_overlay_style(
        field: ProviderModelConfigField,
    ) -> ChoiceOverlayStyle {
        match field {
            ProviderModelConfigField::Enabled
            | ProviderModelConfigField::AgenaToolTransport
            | ProviderModelConfigField::ProviderTools => ChoiceOverlayStyle::SelectOnly,
            ProviderModelConfigField::Lifecycle => ChoiceOverlayStyle::Searchable,
            ProviderModelConfigField::ModelId
            | ProviderModelConfigField::DisplayName
            | ProviderModelConfigField::ContextWindowTokens
            | ProviderModelConfigField::MaxInputTokens
            | ProviderModelConfigField::MaxOutputTokens
            | ProviderModelConfigField::InputModalities
            | ProviderModelConfigField::Features
            | ProviderModelConfigField::OutputModalities
            | ProviderModelConfigField::ThinkingModeVariants
            | ProviderModelConfigField::SpeedModeVariants
            | ProviderModelConfigField::Description => ChoiceOverlayStyle::Searchable,
        }
    }
}
use crate::app::{
    AWS_REGION_CHOICES, App, ChoiceItem, ChoiceOverlayStyle, CredentialIssuer,
    ProviderDraftAuthKind, ProviderDraftSecretSourceKind, ProviderModelConfigField,
    ProviderStudioField, ProviderStudioOverlay, ProviderToolsPreset, RuntimeSettingId,
    RuntimeSettingSpec, SUPPORTED_LOCALES, SettingsFieldKind, SettingsFieldSpec,
    boolean_choice_items, choice_item, choice_item_with_value, inspector_rows_to_mode_choice_items,
    join_inline_segments, provider_studio_adapter_rule, provider_studio_adapter_rule_detail,
    provider_studio_api_key_env_choice_items, provider_studio_default_model_choice_items,
    provider_studio_profile_choice_items, provider_tools_available_preset_for_adapter,
    runtime_setting_choice_parallel_detail, runtime_setting_choice_supported_model_detail,
    settings_choice_adapter_fallback, settings_choice_bool_override_detail,
    settings_choice_default_provider_detail, settings_choice_registered_agent_detail, ui_text,
};

#[cfg(test)]
mod tests {
    use crate::app::{App, ChoiceOverlayStyle, SETTINGS_FIELDS};

    #[test]
    fn registered_default_catalogs_do_not_offer_arbitrary_typed_values() {
        for path in ["providers.default", "agents.default"] {
            let field = SETTINGS_FIELDS
                .iter()
                .copied()
                .find(|field| field.path == path)
                .expect("settings field");
            assert_eq!(
                App::settings_field_choice_overlay_style(field),
                ChoiceOverlayStyle::SearchableSelect
            );
        }
    }

    #[test]
    fn provider_client_versions_accept_typed_exact_versions() {
        for path in [
            "runtime.providers.client_versions.codex",
            "runtime.providers.client_versions.claude",
            "runtime.providers.client_versions.gemini",
        ] {
            let field = SETTINGS_FIELDS
                .iter()
                .copied()
                .find(|field| field.path == path)
                .expect("provider client version settings field");
            assert_eq!(
                App::settings_field_choice_overlay_style(field),
                ChoiceOverlayStyle::Searchable
            );
        }
    }
}
