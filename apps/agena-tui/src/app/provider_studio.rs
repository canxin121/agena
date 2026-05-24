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
        dialog.selection.set_left_selected(index);
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
        .get(dialog.selection.left_selected())
        .cloned()
}

pub(super) fn provider_studio_selected_adapter_models(
    dialog: &ProviderStudioOverlay,
) -> Option<&ProviderAdapterModelsResource> {
    let adapter_id = dialog
        .adapter_candidate_ids
        .get(dialog.selection.left_selected())?;
    dialog
        .adapter_models
        .iter()
        .find(|adapter_models| adapter_models.adapter_id == *adapter_id)
}

pub(super) fn provider_studio_selected_model_target(
    dialog: &ProviderStudioOverlay,
) -> Option<(String, String, Option<ProviderModel>)> {
    let adapter_models = provider_studio_selected_adapter_models(dialog)?;
    let model = adapter_models
        .models
        .get(dialog.selection.right_selected())?
        .clone();
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
    if !default_adapter_valid
        && let Some(adapter_models) = dialog.adapter_models.iter().find(|adapter_models| {
            adapter_models.error.is_none()
                && dialog
                    .selected_adapter_ids
                    .contains(adapter_models.adapter_id.as_str())
                && !provider_studio_selected_models_for_adapter(dialog, adapter_models).is_empty()
        })
    {
        dialog.draft.default_adapter = adapter_models.adapter_id.clone();
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ProviderStudioAuthStatus {
    Pending,
    Unset,
    None,
    SelectIssuer,
    Configured,
    Partial,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProviderStudioSummaryLabel {
    Env,
    Callback,
    Redirect,
    Account,
    Name,
    User,
    Email,
    Profile,
    Region,
    Code,
}

fn provider_studio_summary_label(i18n: &I18n, label: ProviderStudioSummaryLabel) -> String {
    ui_text::t(
        i18n,
        match label {
            ProviderStudioSummaryLabel::Env => "provider-studio-summary-env",
            ProviderStudioSummaryLabel::Callback => "provider-studio-summary-callback",
            ProviderStudioSummaryLabel::Redirect => "provider-studio-summary-redirect",
            ProviderStudioSummaryLabel::Account => "provider-studio-summary-account",
            ProviderStudioSummaryLabel::Name => "provider-studio-summary-name",
            ProviderStudioSummaryLabel::User => "provider-studio-summary-user",
            ProviderStudioSummaryLabel::Email => "provider-studio-summary-email",
            ProviderStudioSummaryLabel::Profile => "provider-studio-summary-profile",
            ProviderStudioSummaryLabel::Region => "provider-studio-summary-region",
            ProviderStudioSummaryLabel::Code => "provider-studio-summary-code",
        },
    )
}

fn provider_studio_labeled_summary(
    i18n: &I18n,
    label: ProviderStudioSummaryLabel,
    value: &str,
    max_width: usize,
) -> Option<String> {
    provider_studio_summary_value(value, max_width)
        .map(|value| format!("{} {value}", provider_studio_summary_label(i18n, label)))
}

pub(super) fn provider_studio_status_with_summary(
    status: String,
    summary: Option<String>,
) -> String {
    let mut parts = vec![status];
    if let Some(summary) = summary {
        parts.push(summary);
    }
    join_inline_segments(parts)
}

fn provider_studio_state_summary(i18n: &I18n, state: &str, max_width: usize) -> Option<String> {
    provider_studio_summary_value(state, max_width).map(|state| {
        i18n.text_args(
            "provider-studio-summary-state",
            &crate::fl_args!("state" => state),
        )
    })
}

pub(super) fn provider_studio_browser_continue_summary(
    i18n: &I18n,
    prefix_key: &str,
    state: &str,
) -> String {
    let mut parts = vec![ui_text::t(i18n, prefix_key)];
    if let Some(state) = provider_studio_state_summary(i18n, state, 20) {
        parts.push(state);
    }
    join_inline_segments(parts)
}

pub(super) fn provider_studio_start_auth_summary(
    i18n: &I18n,
    dialog: &ProviderStudioOverlay,
) -> String {
    let status = provider_studio_auth_status_summary(i18n, dialog);
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
                if let Some(code) = provider_studio_labeled_summary(
                    i18n,
                    ProviderStudioSummaryLabel::Code,
                    device.user_code.as_str(),
                    18,
                ) {
                    parts.push(code);
                }
                (!parts.is_empty()).then(|| join_inline_segments(parts))
            })
            .unwrap_or(status),
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
        Some(CredentialIssuer::GoogleAdc | CredentialIssuer::SapAiCore) | None => status,
    }
}

pub(super) fn provider_studio_continue_auth_summary(
    i18n: &I18n,
    dialog: &ProviderStudioOverlay,
) -> String {
    let status = provider_studio_auth_status_summary(i18n, dialog);
    match dialog.draft.auth_kind.credential_issuer() {
        Some(CredentialIssuer::OpenaiChatgpt) => provider_studio_labeled_summary(
            i18n,
            ProviderStudioSummaryLabel::Callback,
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
                        i18n,
                        "provider-studio-summary-paste-callback",
                        session.state.as_str(),
                    )
                })
        })
        .unwrap_or(status),
        Some(CredentialIssuer::GithubCopilot) => dialog
            .draft
            .credential_drafts
            .github_copilot
            .device
            .as_ref()
            .map(|device| {
                let mut parts = vec![i18n.text_args(
                    "provider-studio-summary-poll-every",
                    &crate::fl_args!("seconds" => device.interval_seconds.max(1) as i64),
                )];
                if let Some(code) = provider_studio_labeled_summary(
                    i18n,
                    ProviderStudioSummaryLabel::Code,
                    device.user_code.as_str(),
                    18,
                ) {
                    parts.push(code);
                }
                join_inline_segments(parts)
            })
            .unwrap_or(status),
        Some(CredentialIssuer::Gitlab) => provider_studio_labeled_summary(
            i18n,
            ProviderStudioSummaryLabel::Callback,
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
                        i18n,
                        "provider-studio-summary-paste-callback",
                        session.state.as_str(),
                    )
                })
        })
        .unwrap_or(status),
        Some(CredentialIssuer::AtomGit) => dialog
            .draft
            .credential_drafts
            .atomgit
            .browser
            .as_ref()
            .map(|session| {
                provider_studio_browser_continue_summary(
                    i18n,
                    "provider-studio-summary-poll-browser",
                    session.state.as_str(),
                )
            })
            .unwrap_or(status),
        Some(CredentialIssuer::GoogleAdc | CredentialIssuer::SapAiCore) | None => status,
    }
}

pub(super) fn provider_studio_auth_details_hint(
    i18n: &I18n,
    draft: &ProviderConfigDraft,
) -> Option<String> {
    match draft.auth_kind {
        ProviderDraftAuthKind::Unset
        | ProviderDraftAuthKind::None
        | ProviderDraftAuthKind::Credential(None) => None,
        ProviderDraftAuthKind::Api => provider_studio_labeled_summary(
            i18n,
            ProviderStudioSummaryLabel::Env,
            draft.auth.api_key_env.as_str(),
            28,
        )
        .or_else(|| provider_studio_summary_value(draft.auth.base_url.as_str(), 48)),
        ProviderDraftAuthKind::Gitlab => {
            provider_studio_summary_value(draft.auth.instance_url.as_str(), 48).or_else(|| {
                provider_studio_labeled_summary(
                    i18n,
                    ProviderStudioSummaryLabel::Env,
                    draft.auth.api_key_env.as_str(),
                    28,
                )
            })
        }
        ProviderDraftAuthKind::Credential(Some(CredentialIssuer::OpenaiChatgpt)) => {
            provider_studio_labeled_summary(
                i18n,
                ProviderStudioSummaryLabel::Account,
                draft.credential_drafts.openai_chatgpt.account_id.as_str(),
                24,
            )
            .or_else(|| {
                provider_studio_labeled_summary(
                    i18n,
                    ProviderStudioSummaryLabel::Callback,
                    draft.credential_drafts.openai_chatgpt.callback_url.as_str(),
                    36,
                )
            })
            .or_else(|| {
                provider_studio_labeled_summary(
                    i18n,
                    ProviderStudioSummaryLabel::Redirect,
                    draft.credential_drafts.openai_chatgpt.redirect_uri.as_str(),
                    36,
                )
            })
            .or_else(|| {
                draft
                    .tokens_present()
                    .then(|| ui_text::t(i18n, "provider-studio-summary-tokens-set"))
            })
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
            .or_else(|| {
                draft
                    .tokens_present()
                    .then(|| ui_text::t(i18n, "provider-studio-summary-tokens-set"))
            })
        }
        ProviderDraftAuthKind::Credential(Some(CredentialIssuer::Gitlab)) => {
            provider_studio_summary_value(draft.auth.instance_url.as_str(), 48)
                .or_else(|| {
                    provider_studio_labeled_summary(
                        i18n,
                        ProviderStudioSummaryLabel::Callback,
                        draft.credential_drafts.gitlab.callback_url.as_str(),
                        36,
                    )
                })
                .or_else(|| {
                    provider_studio_labeled_summary(
                        i18n,
                        ProviderStudioSummaryLabel::Redirect,
                        draft.credential_drafts.gitlab.redirect_uri.as_str(),
                        36,
                    )
                })
                .or_else(|| {
                    draft
                        .tokens_present()
                        .then(|| ui_text::t(i18n, "provider-studio-summary-tokens-set"))
                })
        }
        ProviderDraftAuthKind::Credential(Some(CredentialIssuer::GoogleAdc)) => {
            provider_studio_summary_value(draft.auth.base_url.as_str(), 48)
        }
        ProviderDraftAuthKind::Credential(Some(CredentialIssuer::SapAiCore)) => {
            provider_studio_labeled_summary(
                i18n,
                ProviderStudioSummaryLabel::Env,
                draft.auth.service_key_env.as_str(),
                28,
            )
            .or_else(|| provider_studio_summary_value(draft.auth.base_url.as_str(), 48))
        }
        ProviderDraftAuthKind::Credential(Some(CredentialIssuer::AtomGit)) => {
            provider_studio_labeled_summary(
                i18n,
                ProviderStudioSummaryLabel::Name,
                draft.credential_drafts.atomgit.display_name.as_str(),
                28,
            )
            .or_else(|| {
                provider_studio_labeled_summary(
                    i18n,
                    ProviderStudioSummaryLabel::User,
                    draft.credential_drafts.atomgit.username.as_str(),
                    28,
                )
            })
            .or_else(|| {
                provider_studio_labeled_summary(
                    i18n,
                    ProviderStudioSummaryLabel::Email,
                    draft.credential_drafts.atomgit.email.as_str(),
                    32,
                )
            })
            .or_else(|| {
                provider_studio_labeled_summary(
                    i18n,
                    ProviderStudioSummaryLabel::Account,
                    draft.credential_drafts.atomgit.account_id.as_str(),
                    24,
                )
            })
            .or_else(|| {
                draft
                    .tokens_present()
                    .then(|| ui_text::t(i18n, "provider-studio-summary-tokens-set"))
            })
        }
        ProviderDraftAuthKind::BedrockSigv4 => provider_studio_labeled_summary(
            i18n,
            ProviderStudioSummaryLabel::Profile,
            draft.auth.profile.as_str(),
            24,
        )
        .or_else(|| {
            provider_studio_labeled_summary(
                i18n,
                ProviderStudioSummaryLabel::Region,
                draft.auth.region.as_str(),
                24,
            )
        })
        .or_else(|| provider_studio_summary_value(draft.auth.base_url.as_str(), 48))
        .or_else(|| {
            (!draft.auth.access_key_id.trim().is_empty()
                && !draft.auth.secret_access_key.trim().is_empty())
            .then(|| ui_text::t(i18n, "provider-studio-summary-keys-set"))
        }),
    }
}

pub(super) fn provider_studio_auth_details_summary(
    i18n: &I18n,
    dialog: &ProviderStudioOverlay,
) -> String {
    provider_studio_status_with_summary(
        provider_studio_auth_status_summary(i18n, dialog),
        provider_studio_auth_details_hint(i18n, &dialog.draft),
    )
}

pub(super) fn provider_studio_main_field_value(
    i18n: &I18n,
    dialog: &ProviderStudioOverlay,
    field: ProviderStudioField,
) -> String {
    match field {
        ProviderStudioField::AuthStatus => provider_studio_auth_status_summary(i18n, dialog),
        ProviderStudioField::StartAuthAction => provider_studio_start_auth_summary(i18n, dialog),
        ProviderStudioField::ContinueAuthAction => {
            provider_studio_continue_auth_summary(i18n, dialog)
        }
        ProviderStudioField::EditAuthDetailsAction => {
            provider_studio_auth_details_summary(i18n, dialog)
        }
        _ => provider_studio_field_value(&dialog.draft, field),
    }
}

fn provider_studio_has_pending_auth_state(dialog: &ProviderStudioOverlay) -> bool {
    match dialog.draft.auth_kind.credential_issuer() {
        Some(CredentialIssuer::OpenaiChatgpt) => dialog
            .draft
            .credential_drafts
            .openai_chatgpt
            .browser
            .is_some(),
        Some(CredentialIssuer::GithubCopilot) => dialog
            .draft
            .credential_drafts
            .github_copilot
            .device
            .is_some(),
        Some(CredentialIssuer::Gitlab) => dialog.draft.credential_drafts.gitlab.browser.is_some(),
        Some(CredentialIssuer::AtomGit) => dialog.draft.credential_drafts.atomgit.browser.is_some(),
        Some(CredentialIssuer::GoogleAdc | CredentialIssuer::SapAiCore) | None => false,
    }
}

pub(super) fn provider_studio_auth_state_lines(
    i18n: &I18n,
    dialog: &ProviderStudioOverlay,
) -> Vec<String> {
    match dialog.draft.auth_kind.credential_issuer() {
        Some(CredentialIssuer::OpenaiChatgpt) => dialog
            .draft
            .credential_drafts
            .openai_chatgpt
            .browser
            .as_ref()
            .map(|session| {
                vec![
                    ui_text::t(i18n, "provider-studio-auth-openai-ready"),
                    i18n.text_args(
                        "provider-studio-auth-authorize",
                        &crate::fl_args!(
                            "url" => truncate_display_width(session.authorize_url.as_str(), 56)
                        ),
                    ),
                    i18n.text_args(
                        "provider-studio-auth-paste-callback",
                        &crate::fl_args!(
                            "state" => truncate_display_width(session.state.as_str(), 24)
                        ),
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
                    i18n.text_args(
                        "provider-studio-auth-copilot-ready",
                        &crate::fl_args!("code" => device.user_code.clone()),
                    ),
                    i18n.text_args(
                        "provider-studio-auth-verify",
                        &crate::fl_args!(
                            "url" => truncate_display_width(
                                device.verification_url.as_str(),
                                56,
                            )
                        ),
                    ),
                    i18n.text_args(
                        "provider-studio-auth-poll",
                        &crate::fl_args!("seconds" => device.interval_seconds as i64),
                    ),
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
                    ui_text::t(i18n, "provider-studio-auth-gitlab-ready"),
                    i18n.text_args(
                        "provider-studio-auth-authorize",
                        &crate::fl_args!(
                            "url" => truncate_display_width(session.authorize_url.as_str(), 56)
                        ),
                    ),
                    i18n.text_args(
                        "provider-studio-auth-paste-callback",
                        &crate::fl_args!(
                            "state" => truncate_display_width(session.state.as_str(), 24)
                        ),
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
                    ui_text::t(i18n, "provider-studio-auth-atomgit-ready"),
                    i18n.text_args(
                        "provider-studio-auth-authorize",
                        &crate::fl_args!(
                            "url" => truncate_display_width(session.authorize_url.as_str(), 56)
                        ),
                    ),
                    i18n.text_args(
                        "provider-studio-auth-finish-browser",
                        &crate::fl_args!(
                            "state" => truncate_display_width(session.state.as_str(), 24)
                        ),
                    ),
                ]
            })
            .unwrap_or_default(),
        Some(CredentialIssuer::GoogleAdc | CredentialIssuer::SapAiCore) | None => Vec::new(),
    }
}

pub(super) fn provider_studio_auth_status(
    dialog: &ProviderStudioOverlay,
) -> ProviderStudioAuthStatus {
    if provider_studio_has_pending_auth_state(dialog) {
        return ProviderStudioAuthStatus::Pending;
    }
    let detail_fields = provider_studio_detail_fields(dialog);
    if detail_fields.is_empty() {
        return match dialog.draft.auth_kind {
            ProviderDraftAuthKind::Unset => ProviderStudioAuthStatus::Unset,
            ProviderDraftAuthKind::None => ProviderStudioAuthStatus::None,
            ProviderDraftAuthKind::Credential(None) => ProviderStudioAuthStatus::SelectIssuer,
            ProviderDraftAuthKind::Api
            | ProviderDraftAuthKind::Gitlab
            | ProviderDraftAuthKind::Credential(Some(_))
            | ProviderDraftAuthKind::BedrockSigv4 => ProviderStudioAuthStatus::Unset,
        };
    }
    if provider_studio_auth_is_configured(dialog) {
        ProviderStudioAuthStatus::Configured
    } else if provider_studio_has_any_auth_detail_value(&dialog.draft, &detail_fields) {
        ProviderStudioAuthStatus::Partial
    } else {
        ProviderStudioAuthStatus::Unset
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

pub(super) fn provider_studio_auth_status_summary(
    i18n: &I18n,
    dialog: &ProviderStudioOverlay,
) -> String {
    ui_text::t(
        i18n,
        match provider_studio_auth_status(dialog) {
            ProviderStudioAuthStatus::Pending => "provider-studio-auth-status-pending",
            ProviderStudioAuthStatus::Unset => "provider-studio-auth-status-unset",
            ProviderStudioAuthStatus::None => "provider-studio-auth-status-none",
            ProviderStudioAuthStatus::SelectIssuer => "provider-studio-auth-status-select-issuer",
            ProviderStudioAuthStatus::Configured => "provider-studio-auth-status-configured",
            ProviderStudioAuthStatus::Partial => "provider-studio-auth-status-partial",
        },
    )
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

fn provider_studio_field_label_key(field: ProviderStudioField) -> &'static str {
    match field {
        ProviderStudioField::ProviderId => "provider-field-provider-id",
        ProviderStudioField::AuthMode => "provider-field-auth-mode",
        ProviderStudioField::CredentialIssuer => "provider-field-credential-issuer",
        ProviderStudioField::AuthStatus => "provider-field-auth-status",
        ProviderStudioField::StartAuthAction => "provider-field-start-auth",
        ProviderStudioField::ContinueAuthAction => "provider-field-continue-auth",
        ProviderStudioField::EditAuthDetailsAction => "provider-field-auth-details",
        ProviderStudioField::BaseUrl => "provider-field-base-url",
        ProviderStudioField::InstanceUrl => "provider-field-instance-url",
        ProviderStudioField::ApiKeyEnv => "provider-field-api-key-env",
        ProviderStudioField::ApiKey => "provider-field-api-key",
        ProviderStudioField::RedirectUri => "provider-field-redirect-uri",
        ProviderStudioField::CallbackUrl => "provider-field-callback-url",
        ProviderStudioField::RefreshToken => "provider-field-refresh-token",
        ProviderStudioField::AccessToken => "provider-field-access-token",
        ProviderStudioField::ExpiresAtMs => "provider-field-expires-at-ms",
        ProviderStudioField::AccountId => "provider-field-account-id",
        ProviderStudioField::EnterpriseDomain => "provider-field-enterprise-domain",
        ProviderStudioField::Username => "provider-field-username",
        ProviderStudioField::DisplayName => "provider-field-display-name",
        ProviderStudioField::Email => "provider-field-email",
        ProviderStudioField::AvatarUrl => "provider-field-avatar-url",
        ProviderStudioField::Region => "provider-field-region",
        ProviderStudioField::Profile => "provider-field-profile",
        ProviderStudioField::AccessKeyId => "provider-field-access-key-id",
        ProviderStudioField::SecretAccessKey => "provider-field-secret-access-key",
        ProviderStudioField::SessionToken => "provider-field-session-token",
        ProviderStudioField::ServiceKeyEnv => "provider-field-service-key-env",
        ProviderStudioField::DefaultAdapter => "provider-field-default-adapter",
        ProviderStudioField::DefaultModel => "provider-field-default-model",
    }
}

pub(super) fn provider_studio_field_label(i18n: &I18n, field: ProviderStudioField) -> String {
    ui_text::t(i18n, provider_studio_field_label_key(field))
}

pub(super) fn provider_studio_field_prompt(i18n: &I18n, field: ProviderStudioField) -> String {
    match field {
        ProviderStudioField::AuthMode => {
            ui_text::t(i18n, "overlay-provider-studio-edit-auth-mode-prompt")
        }
        ProviderStudioField::CredentialIssuer => ui_text::t(
            i18n,
            "overlay-provider-studio-edit-credential-issuer-prompt",
        ),
        ProviderStudioField::AuthStatus
        | ProviderStudioField::StartAuthAction
        | ProviderStudioField::ContinueAuthAction
        | ProviderStudioField::EditAuthDetailsAction => String::new(),
        _ => i18n.text_args(
            "overlay-provider-studio-edit-prompt",
            &crate::fl_args!("field" => provider_studio_field_label(i18n, field)),
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
