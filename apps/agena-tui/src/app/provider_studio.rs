use super::*;

pub(super) fn provider_studio_request_key(
    draft: &ProviderConfigDraft,
    adapter_ids: &[String],
) -> String {
    draft.request_fingerprint(adapter_ids)
}

pub(super) fn provider_studio_auth_request_key(
    draft: &ProviderConfigDraft,
    action: &str,
) -> String {
    format!("{action}:{}", provider_studio_request_key(draft, &[]))
}

pub(super) fn provider_studio_candidate_adapter_ids(
    draft: &ProviderConfigDraft,
    configured_adapter_ids: BTreeSet<String>,
) -> Vec<String> {
    let mut adapter_ids = draft
        .auth_kind
        .adapter_rules()
        .iter()
        .map(|rule| rule.adapter_id.to_owned())
        .collect::<Vec<_>>();
    let mut configured_extras = configured_adapter_ids.into_iter().collect::<Vec<_>>();
    configured_extras.sort();
    for adapter_id in configured_extras {
        if !adapter_ids.iter().any(|candidate| candidate == &adapter_id) {
            adapter_ids.push(adapter_id);
        }
    }
    let default_adapter = draft.default_adapter.trim();
    if !default_adapter.is_empty()
        && !adapter_ids
            .iter()
            .any(|candidate| candidate.as_str() == default_adapter)
    {
        adapter_ids.push(default_adapter.to_owned());
    }
    adapter_ids
}

pub(super) fn provider_studio_effective_adapter_ids(
    dialog: &ProviderStudioOverlay,
) -> BTreeSet<String> {
    let mut adapter_ids = dialog.configured_adapter_ids.clone();
    adapter_ids.extend(dialog.selected_adapter_ids.iter().cloned());
    let default_adapter = dialog.draft.default_adapter.trim();
    if !default_adapter.is_empty() {
        adapter_ids.insert(default_adapter.to_owned());
    }
    adapter_ids
}

pub(super) fn provider_studio_adapter_selectable(
    dialog: &ProviderStudioOverlay,
    adapter_id: &str,
) -> bool {
    provider_studio_adapter_rule(dialog, adapter_id).is_some()
}

pub(super) fn provider_studio_request_adapter_ids(dialog: &ProviderStudioOverlay) -> Vec<String> {
    dialog
        .selected_adapter_ids
        .iter()
        .filter(|adapter_id| provider_studio_adapter_selectable(dialog, adapter_id.as_str()))
        .cloned()
        .collect()
}

pub(super) fn restore_provider_studio_adapter_selection(
    dialog: &mut ProviderStudioOverlay,
    selected_adapter_ids: &BTreeSet<String>,
    selected_adapter_id: Option<&str>,
) {
    dialog.selected_adapter_ids = selected_adapter_ids
        .iter()
        .filter(|adapter_id| {
            dialog
                .adapter_candidate_ids
                .iter()
                .any(|candidate| candidate == *adapter_id)
                && provider_studio_adapter_selectable(dialog, adapter_id.as_str())
        })
        .cloned()
        .collect();
    if let Some(adapter_id) = selected_adapter_id
        && let Some(index) = dialog
            .adapter_candidate_ids
            .iter()
            .position(|candidate| candidate == adapter_id)
    {
        dialog.selected_adapter = index;
    }
}

pub(super) fn provider_studio_adapter_rule(
    dialog: &ProviderStudioOverlay,
    adapter_id: &str,
) -> Option<&'static ProviderDraftAdapterRule> {
    dialog.draft.auth_kind.adapter_rule(adapter_id)
}

pub(super) fn provider_studio_base_url_visible(dialog: &ProviderStudioOverlay) -> bool {
    if !dialog.draft.auth.base_url.trim().is_empty() {
        return true;
    }
    match dialog.draft.auth_kind {
        ProviderDraftAuthKind::Unset => false,
        ProviderDraftAuthKind::Api => {
            let effective = provider_studio_effective_adapter_ids(dialog);
            if effective.is_empty() {
                dialog
                    .draft
                    .auth_kind
                    .adapter_rules()
                    .iter()
                    .any(|rule| rule.requires_base_url)
            } else {
                effective
                    .iter()
                    .filter_map(|adapter_id| {
                        provider_studio_adapter_rule(dialog, adapter_id.as_str())
                    })
                    .any(|rule| rule.requires_base_url)
            }
        }
        ProviderDraftAuthKind::Gitlab => false,
        ProviderDraftAuthKind::Credential(Some(issuer)) => issuer.uses_http_endpoint(),
        ProviderDraftAuthKind::Credential(None) => false,
        ProviderDraftAuthKind::BedrockSigv4 => true,
        ProviderDraftAuthKind::None => false,
    }
}

pub(super) fn provider_studio_selected_adapter_id(
    dialog: &ProviderStudioOverlay,
) -> Option<String> {
    dialog
        .adapter_candidate_ids
        .get(dialog.selected_adapter)
        .cloned()
}

pub(super) fn provider_studio_selected_adapter_models(
    dialog: &ProviderStudioOverlay,
) -> Option<&ProviderAdapterModelsResource> {
    let adapter_id = dialog.adapter_candidate_ids.get(dialog.selected_adapter)?;
    dialog
        .adapter_models
        .iter()
        .find(|adapter_models| adapter_models.adapter_id == *adapter_id)
}

pub(super) fn provider_studio_selected_model_target(
    dialog: &ProviderStudioOverlay,
) -> Option<(String, String, Option<ProviderModel>)> {
    let adapter_models = provider_studio_selected_adapter_models(dialog)?;
    let model = adapter_models.models.get(dialog.selected_model)?.clone();
    Some((
        adapter_models.adapter_id.clone(),
        model.id.to_string(),
        Some(model),
    ))
}

pub(super) fn provider_studio_selected_adapter_models_for_save(
    dialog: &ProviderStudioOverlay,
) -> Option<ProviderAdapterModelsResource> {
    let adapter_models = provider_studio_selected_adapter_models(dialog)?.clone();
    let ProviderAdapterModelsResource {
        adapter_id,
        enabled,
        resolved_base_url,
        models,
        error,
    } = adapter_models;
    let selected_models = models
        .into_iter()
        .filter(|model| {
            provider_studio_model_selected(dialog, adapter_id.as_str(), model.id.as_str())
        })
        .collect::<Vec<_>>();
    Some(ProviderAdapterModelsResource {
        adapter_id,
        enabled,
        resolved_base_url,
        models: selected_models,
        error,
    })
}

pub(super) fn provider_studio_model_selected(
    dialog: &ProviderStudioOverlay,
    adapter_id: &str,
    model_id: &str,
) -> bool {
    dialog
        .selected_model_keys
        .contains(provider_studio_model_key(adapter_id, model_id).as_str())
}

pub(super) fn provider_studio_selected_models_for_adapter<'a>(
    dialog: &'a ProviderStudioOverlay,
    adapter_models: &'a ProviderAdapterModelsResource,
) -> Vec<&'a ProviderModel> {
    adapter_models
        .models
        .iter()
        .filter(|model| {
            provider_studio_model_selected(
                dialog,
                adapter_models.adapter_id.as_str(),
                model.id.as_str(),
            )
        })
        .collect()
}

pub(super) fn provider_studio_restore_model_selection(dialog: &mut ProviderStudioOverlay) {
    let available = dialog
        .adapter_models
        .iter()
        .flat_map(|adapter_models| {
            adapter_models.models.iter().map(|model| {
                provider_studio_model_key(adapter_models.adapter_id.as_str(), model.id.as_str())
            })
        })
        .collect::<BTreeSet<_>>();
    dialog
        .selected_model_keys
        .retain(|model_key| available.contains(model_key));
    for adapter_models in &dialog.adapter_models {
        let adapter_selected = dialog
            .selected_adapter_ids
            .contains(adapter_models.adapter_id.as_str());
        if !adapter_selected || adapter_models.error.is_some() {
            continue;
        }
        let has_any = adapter_models.models.iter().any(|model| {
            provider_studio_model_selected(
                dialog,
                adapter_models.adapter_id.as_str(),
                model.id.as_str(),
            )
        });
        if !has_any {
            for model in &adapter_models.models {
                dialog.selected_model_keys.insert(provider_studio_model_key(
                    adapter_models.adapter_id.as_str(),
                    model.id.as_str(),
                ));
            }
        }
    }
}

pub(super) fn provider_studio_first_selected_model<'a>(
    dialog: &'a ProviderStudioOverlay,
    adapter_id: &str,
) -> Option<&'a ProviderModel> {
    dialog
        .adapter_models
        .iter()
        .find(|adapter_models| adapter_models.adapter_id == adapter_id)
        .and_then(|adapter_models| {
            adapter_models
                .models
                .iter()
                .find(|model| provider_studio_model_selected(dialog, adapter_id, model.id.as_str()))
        })
}

pub(super) fn provider_studio_ensure_default_selection(dialog: &mut ProviderStudioOverlay) {
    let default_adapter_valid = dialog.adapter_models.iter().any(|adapter_models| {
        adapter_models.error.is_none()
            && dialog
                .selected_adapter_ids
                .contains(adapter_models.adapter_id.as_str())
            && adapter_models.adapter_id == dialog.draft.default_adapter
            && adapter_models.models.iter().any(|model| {
                provider_studio_model_selected(
                    dialog,
                    adapter_models.adapter_id.as_str(),
                    model.id.as_str(),
                )
            })
    });
    if !default_adapter_valid {
        if let Some(adapter_models) = dialog.adapter_models.iter().find(|adapter_models| {
            adapter_models.error.is_none()
                && dialog
                    .selected_adapter_ids
                    .contains(adapter_models.adapter_id.as_str())
                && !provider_studio_selected_models_for_adapter(dialog, adapter_models).is_empty()
        }) {
            dialog.draft.default_adapter = adapter_models.adapter_id.clone();
        }
    }

    let default_model_valid =
        provider_studio_first_selected_model(dialog, dialog.draft.default_adapter.as_str())
            .is_some_and(|model| model.id.as_str() == dialog.draft.default_model.as_str());
    if !default_model_valid {
        if let Some(model) =
            provider_studio_first_selected_model(dialog, dialog.draft.default_adapter.as_str())
        {
            dialog.draft.default_model = model.id.to_string();
        } else {
            dialog.draft.default_model.clear();
        }
    }
}

pub(super) fn provider_studio_supports_saved_model_listing(draft: &ProviderConfigDraft) -> bool {
    draft.supports_saved_model_listing()
}

pub(super) fn provider_studio_can_request_adapter_models(dialog: &ProviderStudioOverlay) -> bool {
    if dialog.draft.auth_kind.supports_draft_model_listing() {
        return true;
    }
    dialog.draft.source_provider_id.is_some()
        && provider_studio_supports_saved_model_listing(&dialog.draft)
}

pub(super) fn provider_studio_summary_value(value: &str, max_width: usize) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| truncate_display_width(value, max_width))
}

pub(super) fn provider_studio_labeled_summary(
    label: &str,
    value: &str,
    max_width: usize,
) -> Option<String> {
    provider_studio_summary_value(value, max_width).map(|value| format!("{label} {value}"))
}

pub(super) fn provider_studio_status_with_summary(status: &str, summary: Option<String>) -> String {
    summary
        .map(|summary| format!("{status}  ·  {summary}"))
        .unwrap_or_else(|| status.to_owned())
}

pub(super) fn provider_studio_browser_continue_summary(prefix: &str, state: &str) -> String {
    provider_studio_summary_value(state, 20)
        .map(|state| format!("{prefix}  ·  state {state}"))
        .unwrap_or_else(|| prefix.to_owned())
}

pub(super) fn provider_studio_start_auth_summary(dialog: &ProviderStudioOverlay) -> String {
    let status = provider_studio_auth_status_summary(dialog);
    match dialog.draft.auth_kind.credential_issuer() {
        Some(CredentialIssuer::OpenaiChatgpt) => dialog
            .draft
            .credential_drafts
            .openai_chatgpt
            .browser
            .as_ref()
            .and_then(|session| provider_studio_summary_value(session.authorize_url.as_str(), 56))
            .unwrap_or_else(|| status.to_owned()),
        Some(CredentialIssuer::GithubCopilot) => dialog
            .draft
            .credential_drafts
            .github_copilot
            .device
            .as_ref()
            .and_then(|device| {
                let mut parts = Vec::new();
                if let Some(url) =
                    provider_studio_summary_value(device.verification_url.as_str(), 40)
                {
                    parts.push(url);
                }
                if let Some(code) = provider_studio_summary_value(device.user_code.as_str(), 18) {
                    parts.push(format!("code {code}"));
                }
                (!parts.is_empty()).then(|| parts.join("  ·  "))
            })
            .unwrap_or_else(|| status.to_owned()),
        Some(CredentialIssuer::Gitlab) => dialog
            .draft
            .credential_drafts
            .gitlab
            .browser
            .as_ref()
            .and_then(|session| provider_studio_summary_value(session.authorize_url.as_str(), 56))
            .unwrap_or_else(|| status.to_owned()),
        Some(CredentialIssuer::AtomGit) => dialog
            .draft
            .credential_drafts
            .atomgit
            .browser
            .as_ref()
            .and_then(|session| provider_studio_summary_value(session.authorize_url.as_str(), 56))
            .unwrap_or_else(|| status.to_owned()),
        Some(CredentialIssuer::GoogleAdc | CredentialIssuer::SapAiCore) | None => status.to_owned(),
    }
}

pub(super) fn provider_studio_continue_auth_summary(dialog: &ProviderStudioOverlay) -> String {
    let status = provider_studio_auth_status_summary(dialog);
    match dialog.draft.auth_kind.credential_issuer() {
        Some(CredentialIssuer::OpenaiChatgpt) => provider_studio_labeled_summary(
            "callback",
            dialog
                .draft
                .credential_drafts
                .openai_chatgpt
                .callback_url
                .as_str(),
            44,
        )
        .or_else(|| {
            dialog
                .draft
                .credential_drafts
                .openai_chatgpt
                .browser
                .as_ref()
                .map(|session| {
                    provider_studio_browser_continue_summary(
                        "paste callback_url",
                        session.state.as_str(),
                    )
                })
        })
        .unwrap_or_else(|| status.to_owned()),
        Some(CredentialIssuer::GithubCopilot) => dialog
            .draft
            .credential_drafts
            .github_copilot
            .device
            .as_ref()
            .map(|device| {
                let mut parts = vec![format!("poll every {}s", device.interval_seconds.max(1))];
                if let Some(code) = provider_studio_summary_value(device.user_code.as_str(), 18) {
                    parts.push(format!("code {code}"));
                }
                parts.join("  ·  ")
            })
            .unwrap_or_else(|| status.to_owned()),
        Some(CredentialIssuer::Gitlab) => provider_studio_labeled_summary(
            "callback",
            dialog.draft.credential_drafts.gitlab.callback_url.as_str(),
            44,
        )
        .or_else(|| {
            dialog
                .draft
                .credential_drafts
                .gitlab
                .browser
                .as_ref()
                .map(|session| {
                    provider_studio_browser_continue_summary(
                        "paste callback_url",
                        session.state.as_str(),
                    )
                })
        })
        .unwrap_or_else(|| status.to_owned()),
        Some(CredentialIssuer::AtomGit) => dialog
            .draft
            .credential_drafts
            .atomgit
            .browser
            .as_ref()
            .map(|session| {
                provider_studio_browser_continue_summary(
                    "poll browser result",
                    session.state.as_str(),
                )
            })
            .unwrap_or_else(|| status.to_owned()),
        Some(CredentialIssuer::GoogleAdc | CredentialIssuer::SapAiCore) | None => status.to_owned(),
    }
}

pub(super) fn provider_studio_auth_details_hint(draft: &ProviderConfigDraft) -> Option<String> {
    match draft.auth_kind {
        ProviderDraftAuthKind::Unset
        | ProviderDraftAuthKind::None
        | ProviderDraftAuthKind::Credential(None) => None,
        ProviderDraftAuthKind::Api => {
            provider_studio_labeled_summary("env", draft.auth.api_key_env.as_str(), 28)
                .or_else(|| provider_studio_summary_value(draft.auth.base_url.as_str(), 48))
        }
        ProviderDraftAuthKind::Gitlab => {
            provider_studio_summary_value(draft.auth.instance_url.as_str(), 48).or_else(|| {
                provider_studio_labeled_summary("env", draft.auth.api_key_env.as_str(), 28)
            })
        }
        ProviderDraftAuthKind::Credential(Some(CredentialIssuer::OpenaiChatgpt)) => {
            provider_studio_labeled_summary(
                "account",
                draft.credential_drafts.openai_chatgpt.account_id.as_str(),
                24,
            )
            .or_else(|| {
                provider_studio_labeled_summary(
                    "callback",
                    draft.credential_drafts.openai_chatgpt.callback_url.as_str(),
                    36,
                )
            })
            .or_else(|| {
                provider_studio_labeled_summary(
                    "redirect",
                    draft.credential_drafts.openai_chatgpt.redirect_uri.as_str(),
                    36,
                )
            })
            .or_else(|| draft.tokens_present().then(|| "tokens set".to_owned()))
        }
        ProviderDraftAuthKind::Credential(Some(CredentialIssuer::GithubCopilot)) => {
            provider_studio_summary_value(
                draft
                    .credential_drafts
                    .github_copilot
                    .enterprise_domain
                    .as_str(),
                32,
            )
            .or_else(|| draft.tokens_present().then(|| "tokens set".to_owned()))
        }
        ProviderDraftAuthKind::Credential(Some(CredentialIssuer::Gitlab)) => {
            provider_studio_summary_value(draft.auth.instance_url.as_str(), 48)
                .or_else(|| {
                    provider_studio_labeled_summary(
                        "callback",
                        draft.credential_drafts.gitlab.callback_url.as_str(),
                        36,
                    )
                })
                .or_else(|| {
                    provider_studio_labeled_summary(
                        "redirect",
                        draft.credential_drafts.gitlab.redirect_uri.as_str(),
                        36,
                    )
                })
                .or_else(|| draft.tokens_present().then(|| "tokens set".to_owned()))
        }
        ProviderDraftAuthKind::Credential(Some(CredentialIssuer::GoogleAdc)) => {
            provider_studio_summary_value(draft.auth.base_url.as_str(), 48)
        }
        ProviderDraftAuthKind::Credential(Some(CredentialIssuer::SapAiCore)) => {
            provider_studio_labeled_summary("env", draft.auth.service_key_env.as_str(), 28)
                .or_else(|| provider_studio_summary_value(draft.auth.base_url.as_str(), 48))
        }
        ProviderDraftAuthKind::Credential(Some(CredentialIssuer::AtomGit)) => {
            provider_studio_labeled_summary(
                "name",
                draft.credential_drafts.atomgit.display_name.as_str(),
                28,
            )
            .or_else(|| {
                provider_studio_labeled_summary(
                    "user",
                    draft.credential_drafts.atomgit.username.as_str(),
                    28,
                )
            })
            .or_else(|| {
                provider_studio_labeled_summary(
                    "email",
                    draft.credential_drafts.atomgit.email.as_str(),
                    32,
                )
            })
            .or_else(|| {
                provider_studio_labeled_summary(
                    "account",
                    draft.credential_drafts.atomgit.account_id.as_str(),
                    24,
                )
            })
            .or_else(|| draft.tokens_present().then(|| "tokens set".to_owned()))
        }
        ProviderDraftAuthKind::BedrockSigv4 => {
            provider_studio_labeled_summary("profile", draft.auth.profile.as_str(), 24)
                .or_else(|| {
                    provider_studio_labeled_summary("region", draft.auth.region.as_str(), 24)
                })
                .or_else(|| provider_studio_summary_value(draft.auth.base_url.as_str(), 48))
                .or_else(|| {
                    (!draft.auth.access_key_id.trim().is_empty()
                        && !draft.auth.secret_access_key.trim().is_empty())
                    .then(|| "keys set".to_owned())
                })
        }
    }
}

pub(super) fn provider_studio_auth_details_summary(dialog: &ProviderStudioOverlay) -> String {
    provider_studio_status_with_summary(
        provider_studio_auth_status_summary(dialog),
        provider_studio_auth_details_hint(&dialog.draft),
    )
}

pub(super) fn provider_studio_main_field_value(
    dialog: &ProviderStudioOverlay,
    field: ProviderStudioField,
) -> String {
    match field {
        ProviderStudioField::AuthStatus => provider_studio_auth_status_summary(dialog).to_owned(),
        ProviderStudioField::StartAuthAction => provider_studio_start_auth_summary(dialog),
        ProviderStudioField::ContinueAuthAction => provider_studio_continue_auth_summary(dialog),
        ProviderStudioField::EditAuthDetailsAction => provider_studio_auth_details_summary(dialog),
        _ => provider_studio_field_value(&dialog.draft, field),
    }
}

pub(super) fn provider_studio_auth_state_lines(dialog: &ProviderStudioOverlay) -> Vec<String> {
    match dialog.draft.auth_kind.credential_issuer() {
        Some(CredentialIssuer::OpenaiChatgpt) => dialog
            .draft
            .credential_drafts
            .openai_chatgpt
            .browser
            .as_ref()
            .map(|session| {
                vec![
                    "oauth browser session ready · open the copied authorize URL".to_owned(),
                    format!(
                        "authorize {}",
                        truncate_display_width(session.authorize_url.as_str(), 56)
                    ),
                    format!(
                        "paste the final callback URL, then press p  ·  state {}",
                        truncate_display_width(session.state.as_str(), 24)
                    ),
                ]
            })
            .unwrap_or_default(),
        Some(CredentialIssuer::GithubCopilot) => dialog
            .draft
            .credential_drafts
            .github_copilot
            .device
            .as_ref()
            .map(|device| {
                vec![
                    format!(
                        "device login ready · open the copied verification URL and enter {}",
                        device.user_code
                    ),
                    format!(
                        "verify {}",
                        truncate_display_width(device.verification_url.as_str(), 56)
                    ),
                    format!("press p to poll  ·  interval {}s", device.interval_seconds),
                ]
            })
            .unwrap_or_default(),
        Some(CredentialIssuer::Gitlab) => dialog
            .draft
            .credential_drafts
            .gitlab
            .browser
            .as_ref()
            .map(|session| {
                vec![
                    "gitlab browser session ready · open the copied authorize URL".to_owned(),
                    format!(
                        "authorize {}",
                        truncate_display_width(session.authorize_url.as_str(), 56)
                    ),
                    format!(
                        "paste the final callback URL, then press p  ·  state {}",
                        truncate_display_width(session.state.as_str(), 24)
                    ),
                ]
            })
            .unwrap_or_default(),
        Some(CredentialIssuer::AtomGit) => dialog
            .draft
            .credential_drafts
            .atomgit
            .browser
            .as_ref()
            .map(|session| {
                vec![
                    "atomgit browser session ready · open the copied authorize URL".to_owned(),
                    format!(
                        "authorize {}",
                        truncate_display_width(session.authorize_url.as_str(), 56)
                    ),
                    format!(
                        "finish the browser flow, then press p  ·  state {}",
                        truncate_display_width(session.state.as_str(), 24)
                    ),
                ]
            })
            .unwrap_or_default(),
        Some(CredentialIssuer::GoogleAdc | CredentialIssuer::SapAiCore) | None => Vec::new(),
    }
}

pub(super) fn provider_studio_detail_fields(
    dialog: &ProviderStudioOverlay,
) -> Vec<ProviderStudioField> {
    match dialog.draft.auth_kind {
        ProviderDraftAuthKind::Unset | ProviderDraftAuthKind::None => Vec::new(),
        ProviderDraftAuthKind::Api => {
            let mut fields = Vec::new();
            if provider_studio_base_url_visible(dialog) {
                fields.push(ProviderStudioField::BaseUrl);
            }
            fields.extend([ProviderStudioField::ApiKeyEnv, ProviderStudioField::ApiKey]);
            fields
        }
        ProviderDraftAuthKind::Gitlab => vec![
            ProviderStudioField::InstanceUrl,
            ProviderStudioField::ApiKeyEnv,
            ProviderStudioField::ApiKey,
        ],
        ProviderDraftAuthKind::Credential(issuer) => match issuer {
            Some(CredentialIssuer::OpenaiChatgpt) => vec![
                ProviderStudioField::RedirectUri,
                ProviderStudioField::CallbackUrl,
                ProviderStudioField::RefreshToken,
                ProviderStudioField::AccessToken,
                ProviderStudioField::ExpiresAtMs,
                ProviderStudioField::AccountId,
            ],
            Some(CredentialIssuer::GithubCopilot) => vec![
                ProviderStudioField::EnterpriseDomain,
                ProviderStudioField::RefreshToken,
                ProviderStudioField::AccessToken,
                ProviderStudioField::ExpiresAtMs,
            ],
            Some(CredentialIssuer::Gitlab) => vec![
                ProviderStudioField::InstanceUrl,
                ProviderStudioField::RedirectUri,
                ProviderStudioField::CallbackUrl,
                ProviderStudioField::RefreshToken,
                ProviderStudioField::AccessToken,
                ProviderStudioField::ExpiresAtMs,
            ],
            Some(CredentialIssuer::GoogleAdc) => {
                if provider_studio_base_url_visible(dialog) {
                    vec![ProviderStudioField::BaseUrl]
                } else {
                    Vec::new()
                }
            }
            Some(CredentialIssuer::SapAiCore) => {
                let mut fields = Vec::new();
                if provider_studio_base_url_visible(dialog) {
                    fields.push(ProviderStudioField::BaseUrl);
                }
                fields.push(ProviderStudioField::ServiceKeyEnv);
                fields
            }
            Some(CredentialIssuer::AtomGit) => vec![
                ProviderStudioField::RefreshToken,
                ProviderStudioField::AccessToken,
                ProviderStudioField::ExpiresAtMs,
                ProviderStudioField::AccountId,
                ProviderStudioField::Username,
                ProviderStudioField::DisplayName,
                ProviderStudioField::Email,
                ProviderStudioField::AvatarUrl,
            ],
            None => Vec::new(),
        },
        ProviderDraftAuthKind::BedrockSigv4 => vec![
            ProviderStudioField::BaseUrl,
            ProviderStudioField::Region,
            ProviderStudioField::Profile,
            ProviderStudioField::AccessKeyId,
            ProviderStudioField::SecretAccessKey,
            ProviderStudioField::SessionToken,
        ],
    }
}

pub(super) fn provider_studio_has_any_auth_detail_value(
    draft: &ProviderConfigDraft,
    fields: &[ProviderStudioField],
) -> bool {
    fields
        .iter()
        .any(|field| !provider_studio_field_value(draft, *field).trim().is_empty())
}

pub(super) fn provider_studio_auth_is_configured(dialog: &ProviderStudioOverlay) -> bool {
    match dialog.draft.auth_kind {
        ProviderDraftAuthKind::Unset => false,
        ProviderDraftAuthKind::None => true,
        ProviderDraftAuthKind::Api => {
            !dialog.draft.auth.base_url.trim().is_empty()
                && (!dialog.draft.auth.api_key.trim().is_empty()
                    || !dialog.draft.auth.api_key_env.trim().is_empty())
        }
        ProviderDraftAuthKind::Gitlab => {
            !dialog.draft.auth.instance_url.trim().is_empty()
                && (!dialog.draft.auth.api_key.trim().is_empty()
                    || !dialog.draft.auth.api_key_env.trim().is_empty())
        }
        ProviderDraftAuthKind::Credential(Some(CredentialIssuer::OpenaiChatgpt))
        | ProviderDraftAuthKind::Credential(Some(CredentialIssuer::GithubCopilot))
        | ProviderDraftAuthKind::Credential(Some(CredentialIssuer::Gitlab))
        | ProviderDraftAuthKind::Credential(Some(CredentialIssuer::AtomGit)) => {
            dialog.draft.active_tokens().is_some_and(|tokens| {
                !tokens.refresh_token.trim().is_empty() || !tokens.access_token.trim().is_empty()
            })
        }
        ProviderDraftAuthKind::Credential(Some(CredentialIssuer::GoogleAdc)) => {
            !dialog.draft.auth.base_url.trim().is_empty()
        }
        ProviderDraftAuthKind::Credential(Some(CredentialIssuer::SapAiCore)) => {
            !dialog.draft.auth.base_url.trim().is_empty()
                && !dialog.draft.auth.service_key_env.trim().is_empty()
        }
        ProviderDraftAuthKind::Credential(None) => false,
        ProviderDraftAuthKind::BedrockSigv4 => {
            !dialog.draft.auth.base_url.trim().is_empty()
                && !dialog.draft.auth.region.trim().is_empty()
                && ((!dialog.draft.auth.profile.trim().is_empty())
                    || (!dialog.draft.auth.access_key_id.trim().is_empty()
                        && !dialog.draft.auth.secret_access_key.trim().is_empty()))
        }
    }
}

pub(super) fn provider_studio_auth_status_summary(dialog: &ProviderStudioOverlay) -> &'static str {
    if !provider_studio_auth_state_lines(dialog).is_empty() {
        return "pending";
    }
    let detail_fields = provider_studio_detail_fields(dialog);
    if detail_fields.is_empty() {
        return match dialog.draft.auth_kind {
            ProviderDraftAuthKind::Unset => "unset",
            ProviderDraftAuthKind::None => "none",
            ProviderDraftAuthKind::Credential(None) => "select issuer",
            ProviderDraftAuthKind::Api
            | ProviderDraftAuthKind::Gitlab
            | ProviderDraftAuthKind::Credential(Some(_))
            | ProviderDraftAuthKind::BedrockSigv4 => "unset",
        };
    }
    if provider_studio_auth_is_configured(dialog) {
        "configured"
    } else if provider_studio_has_any_auth_detail_value(&dialog.draft, &detail_fields) {
        "partial"
    } else {
        "unset"
    }
}

pub(super) fn provider_studio_visible_fields(
    dialog: &ProviderStudioOverlay,
) -> Vec<ProviderStudioField> {
    let mut fields = vec![
        ProviderStudioField::ProviderId,
        ProviderStudioField::AuthMode,
    ];
    if matches!(dialog.draft.auth_kind, ProviderDraftAuthKind::Credential(_)) {
        fields.push(ProviderStudioField::CredentialIssuer);
    }
    if !provider_studio_detail_fields(dialog).is_empty() {
        if dialog.draft.supports_interactive_auth() {
            fields.push(ProviderStudioField::StartAuthAction);
            fields.push(ProviderStudioField::ContinueAuthAction);
        }
        fields.push(ProviderStudioField::EditAuthDetailsAction);
    }
    if !matches!(
        dialog.draft.auth_kind,
        ProviderDraftAuthKind::Unset | ProviderDraftAuthKind::Credential(None)
    ) {
        fields.extend([
            ProviderStudioField::DefaultAdapter,
            ProviderStudioField::DefaultModel,
        ]);
    }
    fields
}

pub(super) fn provider_studio_field_label(field: ProviderStudioField) -> &'static str {
    match field {
        ProviderStudioField::ProviderId => "provider_id",
        ProviderStudioField::AuthMode => "auth_mode",
        ProviderStudioField::CredentialIssuer => "credential_issuer",
        ProviderStudioField::AuthStatus => "auth_status",
        ProviderStudioField::StartAuthAction => "start_auth",
        ProviderStudioField::ContinueAuthAction => "continue_auth",
        ProviderStudioField::EditAuthDetailsAction => "auth_details",
        ProviderStudioField::BaseUrl => "base_url",
        ProviderStudioField::InstanceUrl => "instance_url",
        ProviderStudioField::ApiKeyEnv => "api_key_env",
        ProviderStudioField::ApiKey => "api_key",
        ProviderStudioField::RedirectUri => "redirect_uri",
        ProviderStudioField::CallbackUrl => "callback_url",
        ProviderStudioField::RefreshToken => "refresh_token",
        ProviderStudioField::AccessToken => "access_token",
        ProviderStudioField::ExpiresAtMs => "expires_at_ms",
        ProviderStudioField::AccountId => "account_id",
        ProviderStudioField::EnterpriseDomain => "enterprise_domain",
        ProviderStudioField::Username => "username",
        ProviderStudioField::DisplayName => "display_name",
        ProviderStudioField::Email => "email",
        ProviderStudioField::AvatarUrl => "avatar_url",
        ProviderStudioField::Region => "region",
        ProviderStudioField::Profile => "profile",
        ProviderStudioField::AccessKeyId => "access_key_id",
        ProviderStudioField::SecretAccessKey => "secret_access_key",
        ProviderStudioField::SessionToken => "session_token",
        ProviderStudioField::ServiceKeyEnv => "service_key_env",
        ProviderStudioField::DefaultAdapter => "default_adapter",
        ProviderStudioField::DefaultModel => "default_model",
    }
}

pub(super) fn provider_studio_field_prompt(i18n: &I18n, field: ProviderStudioField) -> String {
    match field {
        ProviderStudioField::AuthMode => {
            "Update auth_mode (none | api | gitlab_api | credential | bedrock_sigv4)".to_string()
        }
        ProviderStudioField::CredentialIssuer => {
            "Update credential_issuer (openai_chatgpt | github_copilot | gitlab | google_adc | sap_ai_core | atomgit)".to_string()
        }
        ProviderStudioField::AuthStatus
        | ProviderStudioField::StartAuthAction
        | ProviderStudioField::ContinueAuthAction
        | ProviderStudioField::EditAuthDetailsAction => String::new(),
        _ => i18n.text_args(
            "overlay-provider-studio-edit-prompt",
            &crate::fl_args!("field" => provider_studio_field_label(field)),
        ),
    }
}

pub(super) fn provider_studio_field_value(
    draft: &ProviderConfigDraft,
    field: ProviderStudioField,
) -> String {
    match field {
        ProviderStudioField::ProviderId => draft.provider_id.clone(),
        ProviderStudioField::AuthMode => draft.auth_kind.mode_label().to_owned(),
        ProviderStudioField::CredentialIssuer => draft.auth.credential_issuer.clone(),
        ProviderStudioField::AuthStatus => String::new(),
        ProviderStudioField::StartAuthAction
        | ProviderStudioField::ContinueAuthAction
        | ProviderStudioField::EditAuthDetailsAction => String::new(),
        ProviderStudioField::BaseUrl => draft.auth.base_url.clone(),
        ProviderStudioField::InstanceUrl => draft.auth.instance_url.clone(),
        ProviderStudioField::ApiKeyEnv => draft.auth.api_key_env.clone(),
        ProviderStudioField::ApiKey => draft.auth.api_key.clone(),
        ProviderStudioField::RedirectUri => draft.redirect_uri().unwrap_or_default().to_owned(),
        ProviderStudioField::CallbackUrl => draft.callback_url().unwrap_or_default().to_owned(),
        ProviderStudioField::RefreshToken => draft
            .active_tokens()
            .map(|tokens| tokens.refresh_token.clone())
            .unwrap_or_default(),
        ProviderStudioField::AccessToken => draft
            .active_tokens()
            .map(|tokens| tokens.access_token.clone())
            .unwrap_or_default(),
        ProviderStudioField::ExpiresAtMs => draft
            .active_tokens()
            .map(|tokens| tokens.expires_at_ms.clone())
            .unwrap_or_default(),
        ProviderStudioField::AccountId => draft.account_id().unwrap_or_default().to_owned(),
        ProviderStudioField::EnterpriseDomain => draft
            .credential_drafts
            .github_copilot
            .enterprise_domain
            .clone(),
        ProviderStudioField::Username => draft.credential_drafts.atomgit.username.clone(),
        ProviderStudioField::DisplayName => draft.credential_drafts.atomgit.display_name.clone(),
        ProviderStudioField::Email => draft.credential_drafts.atomgit.email.clone(),
        ProviderStudioField::AvatarUrl => draft.credential_drafts.atomgit.avatar_url.clone(),
        ProviderStudioField::Region => draft.auth.region.clone(),
        ProviderStudioField::Profile => draft.auth.profile.clone(),
        ProviderStudioField::AccessKeyId => draft.auth.access_key_id.clone(),
        ProviderStudioField::SecretAccessKey => draft.auth.secret_access_key.clone(),
        ProviderStudioField::SessionToken => draft.auth.session_token.clone(),
        ProviderStudioField::ServiceKeyEnv => draft.auth.service_key_env.clone(),
        ProviderStudioField::DefaultAdapter => draft.default_adapter.clone(),
        ProviderStudioField::DefaultModel => draft.default_model.clone(),
    }
}

pub(super) fn provider_studio_field_editable(
    dialog: &ProviderStudioOverlay,
    field: ProviderStudioField,
) -> bool {
    match field {
        ProviderStudioField::ProviderId => dialog.draft.source_provider_id.is_none(),
        ProviderStudioField::AuthMode => true,
        ProviderStudioField::CredentialIssuer => {
            matches!(dialog.draft.auth_kind, ProviderDraftAuthKind::Credential(_))
        }
        ProviderStudioField::AuthStatus => false,
        ProviderStudioField::StartAuthAction | ProviderStudioField::ContinueAuthAction => {
            dialog.draft.supports_interactive_auth()
        }
        ProviderStudioField::EditAuthDetailsAction => {
            !provider_studio_detail_fields(dialog).is_empty()
        }
        ProviderStudioField::BaseUrl => match dialog.draft.auth_kind {
            ProviderDraftAuthKind::Unset => false,
            ProviderDraftAuthKind::Api | ProviderDraftAuthKind::BedrockSigv4 => {
                provider_studio_base_url_visible(dialog)
            }
            ProviderDraftAuthKind::Credential(_) => provider_studio_base_url_visible(dialog),
            ProviderDraftAuthKind::Gitlab | ProviderDraftAuthKind::None => false,
        },
        ProviderStudioField::InstanceUrl => matches!(
            dialog.draft.auth_kind,
            ProviderDraftAuthKind::Gitlab
                | ProviderDraftAuthKind::Credential(Some(CredentialIssuer::Gitlab))
        ),
        ProviderStudioField::ApiKeyEnv | ProviderStudioField::ApiKey => {
            matches!(
                dialog.draft.auth_kind,
                ProviderDraftAuthKind::Api | ProviderDraftAuthKind::Gitlab
            )
        }
        ProviderStudioField::RedirectUri => matches!(
            dialog.draft.auth_kind,
            ProviderDraftAuthKind::Credential(Some(
                CredentialIssuer::OpenaiChatgpt | CredentialIssuer::Gitlab
            ))
        ),
        ProviderStudioField::CallbackUrl => matches!(
            dialog.draft.auth_kind,
            ProviderDraftAuthKind::Credential(Some(
                CredentialIssuer::OpenaiChatgpt | CredentialIssuer::Gitlab
            ))
        ),
        ProviderStudioField::RefreshToken
        | ProviderStudioField::AccessToken
        | ProviderStudioField::ExpiresAtMs => matches!(
            dialog.draft.auth_kind,
            ProviderDraftAuthKind::Credential(Some(
                CredentialIssuer::OpenaiChatgpt
                    | CredentialIssuer::GithubCopilot
                    | CredentialIssuer::Gitlab
                    | CredentialIssuer::AtomGit
            ))
        ),
        ProviderStudioField::AccountId => matches!(
            dialog.draft.auth_kind,
            ProviderDraftAuthKind::Credential(Some(
                CredentialIssuer::OpenaiChatgpt | CredentialIssuer::AtomGit
            ))
        ),
        ProviderStudioField::EnterpriseDomain => matches!(
            dialog.draft.auth_kind,
            ProviderDraftAuthKind::Credential(Some(CredentialIssuer::GithubCopilot))
        ),
        ProviderStudioField::Username
        | ProviderStudioField::DisplayName
        | ProviderStudioField::Email
        | ProviderStudioField::AvatarUrl => matches!(
            dialog.draft.auth_kind,
            ProviderDraftAuthKind::Credential(Some(CredentialIssuer::AtomGit))
        ),
        ProviderStudioField::Region
        | ProviderStudioField::Profile
        | ProviderStudioField::AccessKeyId
        | ProviderStudioField::SecretAccessKey
        | ProviderStudioField::SessionToken => {
            matches!(dialog.draft.auth_kind, ProviderDraftAuthKind::BedrockSigv4)
        }
        ProviderStudioField::ServiceKeyEnv => matches!(
            dialog.draft.auth_kind,
            ProviderDraftAuthKind::Credential(Some(issuer)) if issuer.requires_service_key_env()
        ),
        ProviderStudioField::DefaultAdapter | ProviderStudioField::DefaultModel => true,
    }
}

pub(super) fn provider_studio_model_key(adapter_id: &str, model_id: &str) -> String {
    format!("{adapter_id}\u{1f}{model_id}")
}
