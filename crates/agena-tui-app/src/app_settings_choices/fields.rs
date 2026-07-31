impl App {
    pub(crate) fn settings_field_choice_items(
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
            "session.compaction.auto" => Some(boolean_choice_items(
                ui_text::t(
                    &self.i18n,
                    "settings-field-session-compaction-auto-description",
                )
                .as_str(),
            )),
            "plugins.policy.tool_presentation.default_mode" => Some(vec![
                choice_item(
                    "detailed",
                    ui_text::t(&self.i18n, "settings-plugin-default-mode-detailed-detail"),
                ),
                choice_item(
                    "brief",
                    ui_text::t(&self.i18n, "settings-plugin-default-mode-brief-detail"),
                ),
            ]),
            "plugins.policy.ui_presentation.default_mode" => Some(vec![
                choice_item(
                    "detailed",
                    ui_text::t(
                        &self.i18n,
                        "settings-plugin-ui-default-mode-detailed-detail",
                    ),
                ),
                choice_item(
                    "summary",
                    ui_text::t(&self.i18n, "settings-plugin-ui-default-mode-summary-detail"),
                ),
            ]),
            _ => None,
        }
    }

    pub(crate) fn settings_field_choice_overlay_style(
        field: SettingsFieldSpec,
    ) -> agena_tui::choice::ChoicePresentationStyle {
        match field.path {
            "providers.default" => agena_tui::choice::ChoicePresentationStyle::SearchableSelect,
            "ui.locale"
            | "ui.tui.color_scheme"
            | "ui.tui.graphics"
            | "tracing.filter"
            | "tracing.database"
            | "tracing.adapter"
            | "session.compaction.auto"
            | "plugins.policy.tool_presentation.default_mode"
            | "plugins.policy.ui_presentation.default_mode" => {
                agena_tui::choice::ChoicePresentationStyle::SelectOnly
            }
            "ui.tui.theme" => agena_tui::choice::ChoicePresentationStyle::SearchableSelect,
            _ => agena_tui::choice::ChoicePresentationStyle::Searchable,
        }
    }

    pub(crate) fn provider_studio_field_choice_items(
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
            _ => None,
        }
    }

    pub(crate) fn provider_studio_field_choice_overlay_style(
        field: ProviderStudioField,
    ) -> agena_tui::choice::ChoicePresentationStyle {
        match field {
            ProviderStudioField::AuthMode
            | ProviderStudioField::AuthSubtype
            | ProviderStudioField::AuthLoginMethod
            | ProviderStudioField::InstanceUrl
            | ProviderStudioField::RedirectUri
            | ProviderStudioField::ApiKeySource
            | ProviderStudioField::ServiceKeyEnv => {
                agena_tui::choice::ChoicePresentationStyle::SelectOnly
            }
            ProviderStudioField::Region | ProviderStudioField::Profile => {
                agena_tui::choice::ChoicePresentationStyle::Searchable
            }
            _ => agena_tui::choice::ChoicePresentationStyle::Searchable,
        }
    }

    pub(crate) fn provider_model_config_field_choice_items(
        &self,
        _dialog: &ProviderStudioOverlay,
        field: ProviderModelConfigField,
    ) -> Option<Vec<ChoiceItem>> {
        match field {
            ProviderModelConfigField::Enabled => Some(boolean_choice_items(
                ui_text::t(&self.i18n, "provider-model-enabled-detail").as_str(),
            )),
            ProviderModelConfigField::NativeCompaction => Some(boolean_choice_items(
                ui_text::t(&self.i18n, "provider-model-native-compaction-detail").as_str(),
            )),
            ProviderModelConfigField::AgenaToolMode => Some(vec![
                choice_item_with_value(
                    ui_text::t(&self.i18n, "agena-tool-mode-provider-protocol-label"),
                    "provider_protocol",
                    ui_text::t(&self.i18n, "agena-tool-mode-provider-protocol-detail"),
                ),
                choice_item_with_value(
                    ui_text::t(&self.i18n, "agena-tool-mode-prompt-envelope-label"),
                    "prompt_envelope",
                    ui_text::t(&self.i18n, "agena-tool-mode-prompt-envelope-detail"),
                ),
                choice_item_with_value(
                    ui_text::t(&self.i18n, "agena-tool-mode-disabled-label"),
                    "disabled",
                    ui_text::t(&self.i18n, "agena-tool-mode-disabled-detail"),
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
            ProviderModelConfigField::ModelId
            | ProviderModelConfigField::DisplayName
            | ProviderModelConfigField::ContextWindowTokens
            | ProviderModelConfigField::MaxInputTokens
            | ProviderModelConfigField::MaxOutputTokens
            | ProviderModelConfigField::InputModalities
            | ProviderModelConfigField::Features
            | ProviderModelConfigField::OutputModalities
            | ProviderModelConfigField::ThinkingModes
            | ProviderModelConfigField::SpeedModes
            | ProviderModelConfigField::Description => None,
        }
    }

    pub(crate) fn provider_model_config_field_choice_overlay_style(
        field: ProviderModelConfigField,
    ) -> agena_tui::choice::ChoicePresentationStyle {
        match field {
            ProviderModelConfigField::Enabled
            | ProviderModelConfigField::NativeCompaction
            | ProviderModelConfigField::AgenaToolMode => {
                agena_tui::choice::ChoicePresentationStyle::SelectOnly
            }
            ProviderModelConfigField::Lifecycle => {
                agena_tui::choice::ChoicePresentationStyle::Searchable
            }
            ProviderModelConfigField::ModelId
            | ProviderModelConfigField::DisplayName
            | ProviderModelConfigField::ContextWindowTokens
            | ProviderModelConfigField::MaxInputTokens
            | ProviderModelConfigField::MaxOutputTokens
            | ProviderModelConfigField::InputModalities
            | ProviderModelConfigField::Features
            | ProviderModelConfigField::OutputModalities
            | ProviderModelConfigField::ThinkingModes
            | ProviderModelConfigField::SpeedModes
            | ProviderModelConfigField::Description => {
                agena_tui::choice::ChoicePresentationStyle::Searchable
            }
        }
    }
}
use crate::{
    AWS_REGION_CHOICES, App, ChoiceItem, CredentialIssuer, ProviderDraftAuthKind,
    ProviderDraftSecretSourceKind, ProviderModelConfigField, ProviderStudioField,
    ProviderStudioOverlay, SUPPORTED_LOCALES, SettingsFieldSpec, boolean_choice_items, choice_item,
    choice_item_with_value, provider_studio_api_key_env_choice_items,
    provider_studio_profile_choice_items, settings_choice_adapter_fallback,
    settings_choice_default_provider_detail, ui_text,
};

#[cfg(test)]
mod tests {
    use crate::{App, SETTINGS_FIELDS};

    #[test]
    fn registered_default_catalogs_do_not_offer_arbitrary_typed_values() {
        for path in ["providers.default"] {
            let field = SETTINGS_FIELDS
                .iter()
                .copied()
                .find(|field| field.path == path)
                .expect("settings field");
            assert_eq!(
                App::settings_field_choice_overlay_style(field),
                agena_tui::choice::ChoicePresentationStyle::SearchableSelect
            );
        }
    }
}
