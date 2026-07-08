use super::*;
use crate::backend::{
    ProviderDraftSecretSourceKind, provider_native_tools_suggested_preset_for_draft,
};

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
    provider_studio_auto_select_single_adapter(dialog);
    if let Some(adapter_id) = selected_adapter_id
        && let Some(index) = dialog
            .adapter_candidate_ids
            .iter()
            .position(|candidate| candidate == adapter_id)
    {
        dialog.selection.set_left_selected(index);
    }
}

fn provider_studio_auto_select_single_adapter(dialog: &mut ProviderStudioOverlay) {
    let mut selectable = dialog
        .adapter_candidate_ids
        .iter()
        .enumerate()
        .filter(|(_, adapter_id)| provider_studio_adapter_selectable(dialog, adapter_id.as_str()));
    let Some((index, adapter_id)) = selectable.next() else {
        return;
    };
    if selectable.next().is_some() {
        return;
    }
    dialog.selected_adapter_ids = BTreeSet::from([adapter_id.clone()]);
    dialog.selection.set_left_selected(index);
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
        ProviderDraftAuthKind::ApiPending => false,
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
        ProviderDraftAuthKind::ClineApi => false,
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
            provider_studio_model_selected(dialog, adapter_id.as_str(), model.id.as_ref())
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

pub(super) fn provider_studio_restore_model_selection(dialog: &mut ProviderStudioOverlay) {
    let available = dialog
        .adapter_models
        .iter()
        .flat_map(|adapter_models| {
            adapter_models.models.iter().map(|model| {
                provider_studio_model_key(adapter_models.adapter_id.as_str(), model.id.as_ref())
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
                model.id.as_ref(),
            )
        });
        if !has_any {
            for model in &adapter_models.models {
                dialog.selected_model_keys.insert(provider_studio_model_key(
                    adapter_models.adapter_id.as_str(),
                    model.id.as_ref(),
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
                .find(|model| provider_studio_model_selected(dialog, adapter_id, model.id.as_ref()))
        })
}

pub(super) fn provider_studio_ensure_default_selection(dialog: &mut ProviderStudioOverlay) {
    provider_studio_auto_select_single_adapter(dialog);

    let default_adapter_valid = dialog
        .selected_adapter_ids
        .contains(dialog.draft.default_adapter.as_str())
        && provider_studio_adapter_selectable(dialog, dialog.draft.default_adapter.as_str());
    if !default_adapter_valid {
        dialog.draft.default_adapter.clear();
        dialog.draft.default_model.clear();
        return;
    }

    let default_model_valid =
        provider_studio_first_selected_model(dialog, dialog.draft.default_adapter.as_str())
            .is_some_and(|model| model.id.as_ref() == dialog.draft.default_model.as_str());
    if !default_model_valid {
        dialog.draft.default_model.clear();
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
    SelectSubtype,
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

fn provider_studio_action_with_summary(
    i18n: &I18n,
    action_key: &str,
    summary: Option<String>,
) -> String {
    let mut parts = vec![ui_text::t(i18n, action_key)];
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

fn provider_studio_required_field_summary(i18n: &I18n, field: ProviderStudioField) -> String {
    i18n.text_args(
        "provider-studio-summary-set-field",
        &crate::fl_args!("field" => provider_studio_field_label(i18n, field)),
    )
}

pub(super) fn provider_studio_auth_login_kind(
    dialog: &ProviderStudioOverlay,
) -> Option<ProviderDraftInteractiveLoginKind> {
    dialog.draft.interactive_login_kind()
}

pub(super) fn provider_studio_auth_poll_interval(
    dialog: &ProviderStudioOverlay,
) -> Option<Duration> {
    match dialog.draft.auth_kind.credential_issuer() {
        Some(CredentialIssuer::OpenaiChatgpt) => {
            if provider_studio_auth_login_kind(dialog)
                != Some(ProviderDraftInteractiveLoginKind::Device)
            {
                return None;
            }
            dialog
                .draft
                .credential_drafts
                .openai_chatgpt
                .device
                .as_ref()
                .map(|device| Duration::from_secs(device.interval_seconds.max(1)))
        }
        Some(CredentialIssuer::GithubCopilot) => dialog
            .draft
            .credential_drafts
            .github_copilot
            .device
            .as_ref()
            .map(|device| Duration::from_secs(device.interval_seconds.max(1))),
        Some(CredentialIssuer::Gitlab)
        | Some(CredentialIssuer::GoogleAdc | CredentialIssuer::SapAiCore)
        | None => None,
    }
}

pub(super) fn provider_studio_available_login_kinds(
    dialog: &ProviderStudioOverlay,
) -> Vec<ProviderDraftInteractiveLoginKind> {
    match dialog.draft.auth_kind.credential_issuer() {
        Some(CredentialIssuer::OpenaiChatgpt) => vec![
            ProviderDraftInteractiveLoginKind::Device,
            ProviderDraftInteractiveLoginKind::Browser,
        ],
        Some(CredentialIssuer::GithubCopilot) => vec![ProviderDraftInteractiveLoginKind::Device],
        Some(CredentialIssuer::Gitlab) => vec![ProviderDraftInteractiveLoginKind::Browser],
        Some(CredentialIssuer::GoogleAdc | CredentialIssuer::SapAiCore) | None => Vec::new(),
    }
}

pub(super) fn provider_studio_auth_login_kind_label(
    i18n: &I18n,
    kind: ProviderDraftInteractiveLoginKind,
) -> String {
    ui_text::t(
        i18n,
        match kind {
            ProviderDraftInteractiveLoginKind::Browser => "provider-auth-login-kind-browser-label",
            ProviderDraftInteractiveLoginKind::Device => "provider-auth-login-kind-device-label",
        },
    )
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

pub(super) fn provider_studio_detail_field_index(
    dialog: &ProviderStudioOverlay,
    field: ProviderStudioField,
) -> Option<usize> {
    provider_studio_detail_fields(dialog)
        .iter()
        .position(|candidate| *candidate == field)
}

pub(super) fn provider_studio_missing_start_auth_field(
    dialog: &ProviderStudioOverlay,
) -> Option<ProviderStudioField> {
    match dialog.draft.auth_kind.credential_issuer() {
        Some(CredentialIssuer::OpenaiChatgpt) => (provider_studio_auth_login_kind(dialog)
            == Some(ProviderDraftInteractiveLoginKind::Browser)
            && dialog
                .draft
                .credential_drafts
                .openai_chatgpt
                .redirect_uri
                .trim()
                .is_empty())
        .then_some(ProviderStudioField::RedirectUri),
        Some(CredentialIssuer::GithubCopilot) => None,
        Some(CredentialIssuer::Gitlab) => {
            if dialog.draft.auth.instance_url.trim().is_empty() {
                Some(ProviderStudioField::InstanceUrl)
            } else {
                dialog
                    .draft
                    .credential_drafts
                    .gitlab
                    .redirect_uri
                    .trim()
                    .is_empty()
                    .then_some(ProviderStudioField::RedirectUri)
            }
        }
        Some(CredentialIssuer::GoogleAdc | CredentialIssuer::SapAiCore) | None => None,
    }
}

pub(super) fn provider_studio_missing_continue_auth_field(
    dialog: &ProviderStudioOverlay,
) -> Option<ProviderStudioField> {
    match dialog.draft.auth_kind.credential_issuer() {
        Some(CredentialIssuer::OpenaiChatgpt) => {
            if provider_studio_auth_login_kind(dialog)
                == Some(ProviderDraftInteractiveLoginKind::Browser)
            {
                if dialog
                    .draft
                    .credential_drafts
                    .openai_chatgpt
                    .redirect_uri
                    .trim()
                    .is_empty()
                {
                    Some(ProviderStudioField::RedirectUri)
                } else {
                    dialog
                        .draft
                        .credential_drafts
                        .openai_chatgpt
                        .browser
                        .as_ref()
                        .and_then(|_| {
                            dialog
                                .draft
                                .credential_drafts
                                .openai_chatgpt
                                .callback_url
                                .trim()
                                .is_empty()
                                .then_some(ProviderStudioField::CallbackUrl)
                        })
                }
            } else {
                None
            }
        }
        Some(CredentialIssuer::GithubCopilot) => None,
        Some(CredentialIssuer::Gitlab) => {
            if dialog.draft.auth.instance_url.trim().is_empty() {
                Some(ProviderStudioField::InstanceUrl)
            } else if dialog
                .draft
                .credential_drafts
                .gitlab
                .redirect_uri
                .trim()
                .is_empty()
            {
                Some(ProviderStudioField::RedirectUri)
            } else {
                dialog
                    .draft
                    .credential_drafts
                    .gitlab
                    .browser
                    .as_ref()
                    .and_then(|_| {
                        dialog
                            .draft
                            .credential_drafts
                            .gitlab
                            .callback_url
                            .trim()
                            .is_empty()
                            .then_some(ProviderStudioField::CallbackUrl)
                    })
            }
        }
        Some(CredentialIssuer::GoogleAdc | CredentialIssuer::SapAiCore) | None => None,
    }
}

pub(super) fn provider_studio_preferred_detail_field_index(
    dialog: &ProviderStudioOverlay,
) -> usize {
    provider_studio_missing_continue_auth_field(dialog)
        .and_then(|field| provider_studio_detail_field_index(dialog, field))
        .or_else(|| match dialog.draft.auth_kind.credential_issuer() {
            Some(CredentialIssuer::OpenaiChatgpt)
                if provider_studio_auth_login_kind(dialog)
                    == Some(ProviderDraftInteractiveLoginKind::Browser) =>
            {
                dialog
                    .draft
                    .credential_drafts
                    .openai_chatgpt
                    .browser
                    .as_ref()
                    .and_then(|_| {
                        provider_studio_detail_field_index(dialog, ProviderStudioField::CallbackUrl)
                    })
            }
            Some(CredentialIssuer::OpenaiChatgpt) => None,
            Some(CredentialIssuer::Gitlab) => dialog
                .draft
                .credential_drafts
                .gitlab
                .browser
                .as_ref()
                .and_then(|_| {
                    provider_studio_detail_field_index(dialog, ProviderStudioField::CallbackUrl)
                }),
            Some(CredentialIssuer::GithubCopilot)
            | Some(CredentialIssuer::GoogleAdc | CredentialIssuer::SapAiCore)
            | None => None,
        })
        .or_else(|| {
            provider_studio_missing_start_auth_field(dialog)
                .and_then(|field| provider_studio_detail_field_index(dialog, field))
        })
        .unwrap_or(0)
}

pub(super) fn provider_studio_start_auth_summary(
    i18n: &I18n,
    dialog: &ProviderStudioOverlay,
) -> String {
    match dialog.draft.auth_kind.credential_issuer() {
        Some(CredentialIssuer::OpenaiChatgpt) => {
            if let Some(field) = provider_studio_missing_start_auth_field(dialog) {
                return provider_studio_required_field_summary(i18n, field);
            }
            match provider_studio_auth_login_kind(dialog) {
                Some(ProviderDraftInteractiveLoginKind::Browser) => {
                    if let Some(session) = dialog
                        .draft
                        .credential_drafts
                        .openai_chatgpt
                        .browser
                        .as_ref()
                    {
                        return provider_studio_action_with_summary(
                            i18n,
                            "provider-studio-summary-open-authorize",
                            Some(session.display_authorize_url().to_owned()),
                        );
                    }
                    provider_studio_action_with_summary(
                        i18n,
                        if provider_studio_auth_is_configured(dialog) {
                            "provider-studio-summary-restart-browser"
                        } else {
                            "provider-studio-summary-start-browser"
                        },
                        provider_studio_labeled_summary(
                            i18n,
                            ProviderStudioSummaryLabel::Redirect,
                            dialog
                                .draft
                                .credential_drafts
                                .openai_chatgpt
                                .redirect_uri
                                .as_str(),
                            36,
                        ),
                    )
                }
                Some(ProviderDraftInteractiveLoginKind::Device) => {
                    if let Some(device) = dialog
                        .draft
                        .credential_drafts
                        .openai_chatgpt
                        .device
                        .as_ref()
                    {
                        let mut parts = vec![device.display_verification_url().to_owned()];
                        if let Some(code) = provider_studio_labeled_summary(
                            i18n,
                            ProviderStudioSummaryLabel::Code,
                            device.user_code.as_str(),
                            18,
                        ) {
                            parts.push(code);
                        }
                        return provider_studio_action_with_summary(
                            i18n,
                            "provider-studio-summary-open-verify",
                            Some(join_inline_segments(parts)),
                        );
                    }
                    provider_studio_action_with_summary(
                        i18n,
                        if provider_studio_auth_is_configured(dialog) {
                            "provider-studio-summary-restart-device"
                        } else {
                            "provider-studio-summary-start-device"
                        },
                        None,
                    )
                }
                None => provider_studio_auth_status_summary(i18n, dialog),
            }
        }
        Some(CredentialIssuer::GithubCopilot) => {
            if let Some(device) = dialog
                .draft
                .credential_drafts
                .github_copilot
                .device
                .as_ref()
            {
                let mut parts = vec![device.display_verification_url().to_owned()];
                if let Some(code) = provider_studio_labeled_summary(
                    i18n,
                    ProviderStudioSummaryLabel::Code,
                    device.user_code.as_str(),
                    18,
                ) {
                    parts.push(code);
                }
                return provider_studio_action_with_summary(
                    i18n,
                    "provider-studio-summary-open-verify",
                    Some(join_inline_segments(parts)),
                );
            }
            provider_studio_action_with_summary(
                i18n,
                if provider_studio_auth_is_configured(dialog) {
                    "provider-studio-summary-restart-device"
                } else {
                    "provider-studio-summary-start-device"
                },
                provider_studio_summary_value(
                    dialog
                        .draft
                        .credential_drafts
                        .github_copilot
                        .enterprise_domain
                        .as_str(),
                    28,
                ),
            )
        }
        Some(CredentialIssuer::Gitlab) => {
            if let Some(field) = provider_studio_missing_start_auth_field(dialog) {
                return provider_studio_required_field_summary(i18n, field);
            }
            if let Some(session) = dialog.draft.credential_drafts.gitlab.browser.as_ref() {
                return provider_studio_action_with_summary(
                    i18n,
                    "provider-studio-summary-open-authorize",
                    Some(session.display_authorize_url().to_owned()),
                );
            }
            provider_studio_action_with_summary(
                i18n,
                if provider_studio_auth_is_configured(dialog) {
                    "provider-studio-summary-restart-browser"
                } else {
                    "provider-studio-summary-start-browser"
                },
                provider_studio_summary_value(dialog.draft.auth.instance_url.as_str(), 40).or_else(
                    || {
                        provider_studio_labeled_summary(
                            i18n,
                            ProviderStudioSummaryLabel::Redirect,
                            dialog.draft.credential_drafts.gitlab.redirect_uri.as_str(),
                            36,
                        )
                    },
                ),
            )
        }
        Some(CredentialIssuer::GoogleAdc | CredentialIssuer::SapAiCore) | None => {
            provider_studio_auth_status_summary(i18n, dialog)
        }
    }
}

pub(super) fn provider_studio_continue_auth_summary(
    i18n: &I18n,
    dialog: &ProviderStudioOverlay,
) -> String {
    match dialog.draft.auth_kind.credential_issuer() {
        Some(CredentialIssuer::OpenaiChatgpt) => match provider_studio_auth_login_kind(dialog) {
            Some(ProviderDraftInteractiveLoginKind::Browser) => {
                if let Some(callback) = provider_studio_labeled_summary(
                    i18n,
                    ProviderStudioSummaryLabel::Callback,
                    dialog
                        .draft
                        .credential_drafts
                        .openai_chatgpt
                        .callback_url
                        .as_str(),
                    44,
                ) {
                    return provider_studio_action_with_summary(
                        i18n,
                        "provider-studio-summary-finish-callback",
                        Some(callback),
                    );
                }
                if let Some(session) = dialog
                    .draft
                    .credential_drafts
                    .openai_chatgpt
                    .browser
                    .as_ref()
                {
                    return provider_studio_browser_continue_summary(
                        i18n,
                        "provider-studio-summary-paste-callback",
                        session.state.as_str(),
                    );
                }
                provider_studio_missing_continue_auth_field(dialog)
                    .map(|field| provider_studio_required_field_summary(i18n, field))
                    .unwrap_or_else(|| ui_text::t(i18n, "provider-studio-summary-start-auth-first"))
            }
            Some(ProviderDraftInteractiveLoginKind::Device) => {
                if let Some(device) = dialog
                    .draft
                    .credential_drafts
                    .openai_chatgpt
                    .device
                    .as_ref()
                {
                    let mut parts = vec![
                        ui_text::t(i18n, "provider-studio-summary-poll-now"),
                        i18n.text_args(
                            "provider-studio-summary-poll-every",
                            &crate::fl_args!("seconds" => device.interval_seconds.max(1) as i64),
                        ),
                    ];
                    if let Some(code) = provider_studio_labeled_summary(
                        i18n,
                        ProviderStudioSummaryLabel::Code,
                        device.user_code.as_str(),
                        18,
                    ) {
                        parts.push(code);
                    }
                    return join_inline_segments(parts);
                }
                provider_studio_action_with_summary(
                    i18n,
                    if provider_studio_auth_is_configured(dialog) {
                        "provider-studio-summary-restart-device"
                    } else {
                        "provider-studio-summary-start-device"
                    },
                    None,
                )
            }
            None => provider_studio_auth_status_summary(i18n, dialog),
        },
        Some(CredentialIssuer::GithubCopilot) => {
            if let Some(device) = dialog
                .draft
                .credential_drafts
                .github_copilot
                .device
                .as_ref()
            {
                let mut parts = vec![
                    ui_text::t(i18n, "provider-studio-summary-poll-now"),
                    i18n.text_args(
                        "provider-studio-summary-poll-every",
                        &crate::fl_args!("seconds" => device.interval_seconds.max(1) as i64),
                    ),
                ];
                if let Some(code) = provider_studio_labeled_summary(
                    i18n,
                    ProviderStudioSummaryLabel::Code,
                    device.user_code.as_str(),
                    18,
                ) {
                    parts.push(code);
                }
                return join_inline_segments(parts);
            }
            provider_studio_action_with_summary(
                i18n,
                if provider_studio_auth_is_configured(dialog) {
                    "provider-studio-summary-restart-device"
                } else {
                    "provider-studio-summary-start-device"
                },
                None,
            )
        }
        Some(CredentialIssuer::Gitlab) => {
            if let Some(callback) = provider_studio_labeled_summary(
                i18n,
                ProviderStudioSummaryLabel::Callback,
                dialog.draft.credential_drafts.gitlab.callback_url.as_str(),
                44,
            ) {
                return provider_studio_action_with_summary(
                    i18n,
                    "provider-studio-summary-finish-callback",
                    Some(callback),
                );
            }
            if let Some(session) = dialog.draft.credential_drafts.gitlab.browser.as_ref() {
                return provider_studio_browser_continue_summary(
                    i18n,
                    "provider-studio-summary-paste-callback",
                    session.state.as_str(),
                );
            }
            provider_studio_missing_continue_auth_field(dialog)
                .map(|field| provider_studio_required_field_summary(i18n, field))
                .unwrap_or_else(|| ui_text::t(i18n, "provider-studio-summary-start-auth-first"))
        }
        Some(CredentialIssuer::GoogleAdc | CredentialIssuer::SapAiCore) | None => {
            provider_studio_auth_status_summary(i18n, dialog)
        }
    }
}

pub(super) fn provider_studio_auth_details_hint(
    i18n: &I18n,
    draft: &ProviderConfigDraft,
) -> Option<String> {
    match draft.auth_kind {
        ProviderDraftAuthKind::Unset
        | ProviderDraftAuthKind::None
        | ProviderDraftAuthKind::ApiPending
        | ProviderDraftAuthKind::Credential(None) => None,
        ProviderDraftAuthKind::Api => provider_studio_secret_source_hint(i18n, draft)
            .or_else(|| provider_studio_summary_value(draft.auth.base_url.as_str(), 48)),
        ProviderDraftAuthKind::ClineApi => provider_studio_secret_source_hint(i18n, draft),
        ProviderDraftAuthKind::Gitlab => {
            provider_studio_summary_value(draft.auth.instance_url.as_str(), 48)
                .or_else(|| provider_studio_secret_source_hint(i18n, draft))
        }
        ProviderDraftAuthKind::Credential(Some(CredentialIssuer::OpenaiChatgpt)) => {
            provider_studio_labeled_summary(
                i18n,
                ProviderStudioSummaryLabel::Account,
                draft.credential_drafts.openai_chatgpt.account_id.as_str(),
                24,
            )
            .or_else(|| {
                (draft.interactive_login_kind() == Some(ProviderDraftInteractiveLoginKind::Browser))
                    .then(|| {
                        provider_studio_labeled_summary(
                            i18n,
                            ProviderStudioSummaryLabel::Callback,
                            draft.credential_drafts.openai_chatgpt.callback_url.as_str(),
                            36,
                        )
                    })
                    .flatten()
            })
            .or_else(|| {
                (draft.interactive_login_kind() == Some(ProviderDraftInteractiveLoginKind::Browser))
                    .then(|| {
                        provider_studio_labeled_summary(
                            i18n,
                            ProviderStudioSummaryLabel::Redirect,
                            draft.credential_drafts.openai_chatgpt.redirect_uri.as_str(),
                            36,
                        )
                    })
                    .flatten()
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

fn provider_studio_secret_source_hint(i18n: &I18n, draft: &ProviderConfigDraft) -> Option<String> {
    match draft.auth.secret_source_kind {
        ProviderDraftSecretSourceKind::Unset => None,
        ProviderDraftSecretSourceKind::Inline => {
            (!draft.auth.secret_source_value.trim().is_empty())
                .then(|| ui_text::t(i18n, "provider-studio-summary-keys-set"))
        }
        ProviderDraftSecretSourceKind::Env => provider_studio_labeled_summary(
            i18n,
            ProviderStudioSummaryLabel::Env,
            draft.auth.secret_source_value.as_str(),
            28,
        ),
    }
}

pub(super) fn provider_studio_auth_details_summary(
    i18n: &I18n,
    dialog: &ProviderStudioOverlay,
) -> String {
    provider_studio_auth_details_hint(i18n, &dialog.draft)
        .unwrap_or_else(|| ui_text::t(i18n, "provider-studio-summary-review-fields"))
}

pub(super) fn provider_studio_main_field_value(
    i18n: &I18n,
    dialog: &ProviderStudioOverlay,
    field: ProviderStudioField,
) -> String {
    match field {
        ProviderStudioField::AuthLoginMethod => dialog
            .draft
            .interactive_login_kind()
            .map(|kind| kind.token().to_owned())
            .unwrap_or_default(),
        ProviderStudioField::StartAuthAction => provider_studio_start_auth_summary(i18n, dialog),
        ProviderStudioField::ContinueAuthAction => {
            provider_studio_continue_auth_summary(i18n, dialog)
        }
        ProviderStudioField::EditAuthDetailsAction => {
            provider_studio_auth_details_summary(i18n, dialog)
        }
        ProviderStudioField::DeleteProviderAction => {
            ui_text::t(i18n, "provider-studio-summary-delete-provider")
        }
        _ => provider_studio_field_value(&dialog.draft, field),
    }
}

fn provider_studio_has_pending_auth_state(dialog: &ProviderStudioOverlay) -> bool {
    match dialog.draft.auth_kind.credential_issuer() {
        Some(CredentialIssuer::OpenaiChatgpt) => match provider_studio_auth_login_kind(dialog) {
            Some(ProviderDraftInteractiveLoginKind::Browser) => dialog
                .draft
                .credential_drafts
                .openai_chatgpt
                .browser
                .is_some(),
            Some(ProviderDraftInteractiveLoginKind::Device) => dialog
                .draft
                .credential_drafts
                .openai_chatgpt
                .device
                .is_some(),
            None => false,
        },
        Some(CredentialIssuer::GithubCopilot) => dialog
            .draft
            .credential_drafts
            .github_copilot
            .device
            .is_some(),
        Some(CredentialIssuer::Gitlab) => dialog.draft.credential_drafts.gitlab.browser.is_some(),
        Some(CredentialIssuer::GoogleAdc | CredentialIssuer::SapAiCore) | None => false,
    }
}

pub(super) fn provider_studio_auth_state_lines(
    i18n: &I18n,
    dialog: &ProviderStudioOverlay,
) -> Vec<String> {
    match dialog.draft.auth_kind.credential_issuer() {
        Some(CredentialIssuer::OpenaiChatgpt) => match provider_studio_auth_login_kind(dialog) {
            Some(ProviderDraftInteractiveLoginKind::Browser) => dialog
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
                            &crate::fl_args!("url" => session.display_authorize_url().to_owned()),
                        ),
                        i18n.text_args(
                            "provider-studio-auth-redirect",
                            &crate::fl_args!(
                                "url" => dialog
                                    .draft
                                    .credential_drafts
                                    .openai_chatgpt
                                    .redirect_uri
                                    .clone()
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
            Some(ProviderDraftInteractiveLoginKind::Device) => dialog
                .draft
                .credential_drafts
                .openai_chatgpt
                .device
                .as_ref()
                .map(|device| {
                    vec![
                        i18n.text_args(
                            "provider-studio-auth-openai-device-ready",
                            &crate::fl_args!("code" => device.user_code.clone()),
                        ),
                        i18n.text_args(
                            "provider-studio-auth-verify",
                            &crate::fl_args!("url" => device.display_verification_url().to_owned()),
                        ),
                        i18n.text_args(
                            "provider-studio-auth-poll",
                            &crate::fl_args!("seconds" => device.interval_seconds.max(1) as i64),
                        ),
                    ]
                })
                .unwrap_or_default(),
            None => Vec::new(),
        },
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
                        &crate::fl_args!("url" => device.display_verification_url().to_owned()),
                    ),
                    i18n.text_args(
                        "provider-studio-auth-poll",
                        &crate::fl_args!("seconds" => device.interval_seconds.max(1) as i64),
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
                        &crate::fl_args!("url" => session.display_authorize_url().to_owned()),
                    ),
                    i18n.text_args(
                        "provider-studio-auth-redirect",
                        &crate::fl_args!(
                            "url" => dialog.draft.credential_drafts.gitlab.redirect_uri.clone()
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
            ProviderDraftAuthKind::ApiPending => ProviderStudioAuthStatus::SelectSubtype,
            ProviderDraftAuthKind::Credential(None) => ProviderStudioAuthStatus::SelectIssuer,
            ProviderDraftAuthKind::Api
            | ProviderDraftAuthKind::ClineApi
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
        ProviderDraftAuthKind::Unset
        | ProviderDraftAuthKind::None
        | ProviderDraftAuthKind::ApiPending => Vec::new(),
        ProviderDraftAuthKind::Api => {
            let mut fields = Vec::new();
            if provider_studio_base_url_visible(dialog) {
                fields.push(ProviderStudioField::BaseUrl);
            }
            fields.extend([
                ProviderStudioField::ApiKeySource,
                ProviderStudioField::ApiKeyValue,
            ]);
            fields
        }
        ProviderDraftAuthKind::ClineApi => {
            vec![
                ProviderStudioField::ApiKeySource,
                ProviderStudioField::ApiKeyValue,
            ]
        }
        ProviderDraftAuthKind::Gitlab => vec![
            ProviderStudioField::InstanceUrl,
            ProviderStudioField::ApiKeySource,
            ProviderStudioField::ApiKeyValue,
        ],
        ProviderDraftAuthKind::Credential(issuer) => match issuer {
            Some(CredentialIssuer::OpenaiChatgpt) => {
                let mut fields = Vec::new();
                if provider_studio_auth_login_kind(dialog)
                    == Some(ProviderDraftInteractiveLoginKind::Browser)
                {
                    fields.push(ProviderStudioField::RedirectUri);
                    fields.push(ProviderStudioField::CallbackUrl);
                }
                fields.extend([
                    ProviderStudioField::RefreshToken,
                    ProviderStudioField::AccessToken,
                    ProviderStudioField::ExpiresAtMs,
                    ProviderStudioField::AccountId,
                ]);
                fields
            }
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
        ProviderDraftAuthKind::ApiPending => false,
        ProviderDraftAuthKind::Api => {
            !dialog.draft.auth.base_url.trim().is_empty()
                && dialog.draft.auth.secret_source_kind != ProviderDraftSecretSourceKind::Unset
                && !dialog.draft.auth.secret_source_value.trim().is_empty()
        }
        ProviderDraftAuthKind::ClineApi => {
            dialog.draft.auth.secret_source_kind != ProviderDraftSecretSourceKind::Unset
                && !dialog.draft.auth.secret_source_value.trim().is_empty()
        }
        ProviderDraftAuthKind::Gitlab => {
            !dialog.draft.auth.instance_url.trim().is_empty()
                && dialog.draft.auth.secret_source_kind != ProviderDraftSecretSourceKind::Unset
                && !dialog.draft.auth.secret_source_value.trim().is_empty()
        }
        ProviderDraftAuthKind::Credential(Some(CredentialIssuer::OpenaiChatgpt))
        | ProviderDraftAuthKind::Credential(Some(CredentialIssuer::GithubCopilot))
        | ProviderDraftAuthKind::Credential(Some(CredentialIssuer::Gitlab)) => {
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
            ProviderStudioAuthStatus::SelectSubtype => "provider-studio-auth-status-select-subtype",
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
    if matches!(
        dialog.draft.auth_kind,
        ProviderDraftAuthKind::ApiPending
            | ProviderDraftAuthKind::Api
            | ProviderDraftAuthKind::ClineApi
            | ProviderDraftAuthKind::Gitlab
            | ProviderDraftAuthKind::BedrockSigv4
            | ProviderDraftAuthKind::Credential(_)
    ) {
        fields.push(ProviderStudioField::AuthSubtype);
    }
    if !provider_studio_available_login_kinds(dialog).is_empty() {
        fields.push(ProviderStudioField::AuthLoginMethod);
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
        ProviderDraftAuthKind::Unset
            | ProviderDraftAuthKind::ApiPending
            | ProviderDraftAuthKind::Credential(None)
    ) {
        fields.extend([
            ProviderStudioField::DefaultAdapter,
            ProviderStudioField::DefaultModel,
        ]);
    }
    if dialog.draft.source_provider_id.is_some() {
        fields.push(ProviderStudioField::DeleteProviderAction);
    }
    fields
}

fn provider_studio_field_label_key(field: ProviderStudioField) -> &'static str {
    match field {
        ProviderStudioField::ProviderId => "provider-field-provider-id",
        ProviderStudioField::AuthMode => "provider-field-auth-mode",
        ProviderStudioField::AuthSubtype => "provider-field-auth-subtype",
        ProviderStudioField::AuthLoginMethod => "provider-field-auth-login-method",
        ProviderStudioField::StartAuthAction => "provider-field-start-auth",
        ProviderStudioField::ContinueAuthAction => "provider-field-continue-auth",
        ProviderStudioField::EditAuthDetailsAction => "provider-field-auth-details",
        ProviderStudioField::DeleteProviderAction => "provider-field-delete-provider",
        ProviderStudioField::BaseUrl => "provider-field-base-url",
        ProviderStudioField::InstanceUrl => "provider-field-instance-url",
        ProviderStudioField::ApiKeySource => "provider-field-api-key-source",
        ProviderStudioField::ApiKeyValue => "provider-field-api-key-value",
        ProviderStudioField::RedirectUri => "provider-field-redirect-uri",
        ProviderStudioField::CallbackUrl => "provider-field-callback-url",
        ProviderStudioField::RefreshToken => "provider-field-refresh-token",
        ProviderStudioField::AccessToken => "provider-field-access-token",
        ProviderStudioField::ExpiresAtMs => "provider-field-expires-at-ms",
        ProviderStudioField::AccountId => "provider-field-account-id",
        ProviderStudioField::EnterpriseDomain => "provider-field-enterprise-domain",
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
        ProviderStudioField::AuthSubtype => {
            ui_text::t(i18n, "overlay-provider-studio-edit-auth-subtype-prompt")
        }
        ProviderStudioField::AuthLoginMethod => ui_text::t(
            i18n,
            "overlay-provider-studio-edit-auth-login-method-prompt",
        ),
        ProviderStudioField::StartAuthAction
        | ProviderStudioField::ContinueAuthAction
        | ProviderStudioField::EditAuthDetailsAction
        | ProviderStudioField::DeleteProviderAction => String::new(),
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
        ProviderStudioField::AuthSubtype => draft.auth_kind.subtype_label().to_owned(),
        ProviderStudioField::AuthLoginMethod => draft
            .interactive_login_kind()
            .map(|kind| kind.token().to_owned())
            .unwrap_or_default(),
        ProviderStudioField::StartAuthAction
        | ProviderStudioField::ContinueAuthAction
        | ProviderStudioField::EditAuthDetailsAction
        | ProviderStudioField::DeleteProviderAction => String::new(),
        ProviderStudioField::BaseUrl => draft.auth.base_url.clone(),
        ProviderStudioField::InstanceUrl => draft.auth.instance_url.clone(),
        ProviderStudioField::ApiKeySource => draft.auth.secret_source_kind.token().to_owned(),
        ProviderStudioField::ApiKeyValue => draft.auth.secret_source_value.clone(),
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
        ProviderStudioField::AuthSubtype => matches!(
            dialog.draft.auth_kind,
            ProviderDraftAuthKind::ApiPending
                | ProviderDraftAuthKind::Api
                | ProviderDraftAuthKind::ClineApi
                | ProviderDraftAuthKind::Gitlab
                | ProviderDraftAuthKind::BedrockSigv4
                | ProviderDraftAuthKind::Credential(_)
        ),
        ProviderStudioField::AuthLoginMethod => {
            provider_studio_available_login_kinds(dialog).len() > 1
        }
        ProviderStudioField::StartAuthAction | ProviderStudioField::ContinueAuthAction => {
            dialog.draft.supports_interactive_auth()
        }
        ProviderStudioField::EditAuthDetailsAction => {
            !provider_studio_detail_fields(dialog).is_empty()
        }
        ProviderStudioField::DeleteProviderAction => dialog.draft.source_provider_id.is_some(),
        ProviderStudioField::BaseUrl => match dialog.draft.auth_kind {
            ProviderDraftAuthKind::Unset => false,
            ProviderDraftAuthKind::ApiPending => false,
            ProviderDraftAuthKind::Api | ProviderDraftAuthKind::BedrockSigv4 => {
                provider_studio_base_url_visible(dialog)
            }
            ProviderDraftAuthKind::ClineApi => false,
            ProviderDraftAuthKind::Credential(_) => provider_studio_base_url_visible(dialog),
            ProviderDraftAuthKind::Gitlab | ProviderDraftAuthKind::None => false,
        },
        ProviderStudioField::InstanceUrl => matches!(
            dialog.draft.auth_kind,
            ProviderDraftAuthKind::Gitlab
                | ProviderDraftAuthKind::Credential(Some(CredentialIssuer::Gitlab))
        ),
        ProviderStudioField::ApiKeySource | ProviderStudioField::ApiKeyValue => {
            matches!(
                dialog.draft.auth_kind,
                ProviderDraftAuthKind::Api
                    | ProviderDraftAuthKind::ClineApi
                    | ProviderDraftAuthKind::Gitlab
            )
        }
        ProviderStudioField::RedirectUri => {
            matches!(
                dialog.draft.auth_kind,
                ProviderDraftAuthKind::Credential(Some(CredentialIssuer::Gitlab))
            ) || matches!(
                dialog.draft.auth_kind,
                ProviderDraftAuthKind::Credential(Some(CredentialIssuer::OpenaiChatgpt))
            ) && provider_studio_auth_login_kind(dialog)
                == Some(ProviderDraftInteractiveLoginKind::Browser)
        }
        ProviderStudioField::CallbackUrl => {
            matches!(
                dialog.draft.auth_kind,
                ProviderDraftAuthKind::Credential(Some(CredentialIssuer::Gitlab))
            ) || matches!(
                dialog.draft.auth_kind,
                ProviderDraftAuthKind::Credential(Some(CredentialIssuer::OpenaiChatgpt))
            ) && provider_studio_auth_login_kind(dialog)
                == Some(ProviderDraftInteractiveLoginKind::Browser)
        }
        ProviderStudioField::RefreshToken
        | ProviderStudioField::AccessToken
        | ProviderStudioField::ExpiresAtMs => matches!(
            dialog.draft.auth_kind,
            ProviderDraftAuthKind::Credential(Some(
                CredentialIssuer::OpenaiChatgpt
                    | CredentialIssuer::GithubCopilot
                    | CredentialIssuer::Gitlab
            ))
        ),
        ProviderStudioField::AccountId => matches!(
            dialog.draft.auth_kind,
            ProviderDraftAuthKind::Credential(Some(CredentialIssuer::OpenaiChatgpt))
        ),
        ProviderStudioField::EnterpriseDomain => matches!(
            dialog.draft.auth_kind,
            ProviderDraftAuthKind::Credential(Some(CredentialIssuer::GithubCopilot))
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

pub(super) fn remove_provider_studio_model_from_dialog(
    dialog: &mut ProviderStudioOverlay,
    adapter_id: &str,
    model_id: &str,
) {
    if let Some(adapter_models) = dialog
        .adapter_models
        .iter_mut()
        .find(|adapter_models| adapter_models.adapter_id == adapter_id)
    {
        adapter_models
            .models
            .retain(|model| model.id.as_ref() != model_id);
    }
    dialog
        .selected_model_keys
        .remove(provider_studio_model_key(adapter_id, model_id).as_str());
    if dialog.draft.default_adapter == adapter_id && dialog.draft.default_model == model_id {
        dialog.draft.default_model.clear();
    }
    dialog.model_page = None;
    dialog.selection.clamp_right(
        provider_studio_selected_adapter_models(dialog)
            .map(|adapter| adapter.models.len())
            .unwrap_or_default(),
    );
    provider_studio_ensure_default_selection(dialog);
}

pub(super) fn remove_provider_studio_adapter_from_dialog(
    dialog: &mut ProviderStudioOverlay,
    adapter_id: &str,
) {
    dialog
        .adapter_models
        .retain(|adapter_models| adapter_models.adapter_id != adapter_id);
    dialog.configured_adapter_ids.remove(adapter_id);
    dialog.selected_adapter_ids.remove(adapter_id);
    let prefix = format!("{adapter_id}\u{1f}");
    dialog
        .selected_model_keys
        .retain(|key| !key.starts_with(prefix.as_str()));
    dialog
        .catalog_matches
        .retain(|key, _| !key.starts_with(prefix.as_str()));
    if dialog.draft.default_adapter == adapter_id {
        dialog.draft.default_adapter.clear();
        dialog.draft.default_model.clear();
    }
    if dialog
        .model_page
        .as_ref()
        .is_some_and(|page| page.adapter_id == adapter_id)
    {
        dialog.model_page = None;
    }
    dialog.selection.clamp_right(
        provider_studio_selected_adapter_models(dialog)
            .map(|adapter| adapter.models.len())
            .unwrap_or_default(),
    );
    provider_studio_ensure_default_selection(dialog);
}

const PROVIDER_MODEL_CONFIG_FIELDS: [ProviderModelConfigField; 12] = [
    ProviderModelConfigField::ModelId,
    ProviderModelConfigField::Enabled,
    ProviderModelConfigField::DisplayName,
    ProviderModelConfigField::Lifecycle,
    ProviderModelConfigField::ContextWindowTokens,
    ProviderModelConfigField::MaxInputTokens,
    ProviderModelConfigField::MaxOutputTokens,
    ProviderModelConfigField::InputModalities,
    ProviderModelConfigField::Features,
    ProviderModelConfigField::OutputModalities,
    ProviderModelConfigField::Description,
    ProviderModelConfigField::NativeTools,
];

pub(super) fn provider_model_config_fields() -> &'static [ProviderModelConfigField] {
    &PROVIDER_MODEL_CONFIG_FIELDS
}

pub(super) fn provider_model_config_draft_from_value(
    model_id: &str,
    value: JsonValue,
) -> std::result::Result<ProviderModelConfigDraft, String> {
    let overlay = serde_json::from_value::<agena::config::ProviderModelOverlay>(value)
        .map_err(|error| error.to_string())?;
    Ok(provider_model_config_draft_from_overlay(model_id, overlay))
}

pub(super) fn apply_provider_model_config_native_tools_suggestion(
    provider_draft: &ProviderConfigDraft,
    adapter_id: &str,
    native_tools_present: bool,
    draft: &mut ProviderModelConfigDraft,
) {
    if native_tools_present || draft.native_tools_preset != ProviderNativeToolsPreset::Disabled {
        return;
    }
    if let Some(preset) =
        provider_native_tools_suggested_preset_for_draft(provider_draft, adapter_id)
    {
        draft.native_tools_preset = preset;
    }
}

pub(super) fn provider_model_config_draft_from_overlay(
    model_id: &str,
    overlay: agena::config::ProviderModelOverlay,
) -> ProviderModelConfigDraft {
    ProviderModelConfigDraft {
        model_id: model_id.to_owned(),
        enabled: overlay.enabled,
        display_name: overlay.definition.display_name.unwrap_or_default(),
        lifecycle: overlay
            .definition
            .lifecycle
            .map(model_lifecycle_token)
            .unwrap_or_default(),
        context_window_tokens: overlay
            .definition
            .context_window_tokens
            .map(|value| value.to_string())
            .unwrap_or_default(),
        max_input_tokens: overlay
            .definition
            .max_input_tokens
            .map(|value| value.to_string())
            .unwrap_or_default(),
        max_output_tokens: overlay
            .definition
            .max_output_tokens
            .map(|value| value.to_string())
            .unwrap_or_default(),
        input_modalities: overlay
            .definition
            .capabilities
            .input
            .as_ref()
            .map(|patch| patch.supported().iter().map(ToString::to_string).collect())
            .unwrap_or_default(),
        features: overlay
            .definition
            .capabilities
            .features
            .as_ref()
            .map(|patch| patch.supported().iter().map(ToString::to_string).collect())
            .unwrap_or_default(),
        output_modalities: overlay.definition.output_modalities.join(", "),
        description: overlay.definition.description.unwrap_or_default(),
        native_tools_preset: provider_native_tools_preset_from_config(&overlay.native_tools),
        native_tools_custom: overlay.native_tools,
    }
}

pub(super) fn provider_model_config_draft_to_model_value(
    draft: &ProviderModelConfigDraft,
) -> std::result::Result<(String, JsonValue), String> {
    let model_id = draft.model_id.trim();
    if model_id.is_empty() {
        return Err("model id is required".to_owned());
    }

    let input = (!draft.input_modalities.is_empty()).then(|| {
        agena::provider::CapabilitySelectionPatch::Supported(
            draft
                .input_modalities
                .iter()
                .filter_map(|value| parse_model_input_modality(value.as_str()))
                .collect(),
        )
    });
    let features = (!draft.features.is_empty()).then(|| {
        agena::provider::CapabilitySelectionPatch::Supported(
            draft
                .features
                .iter()
                .filter_map(|value| parse_model_capability_feature(value.as_str()))
                .collect(),
        )
    });
    let overlay = agena::config::ProviderModelOverlay {
        enabled: draft.enabled,
        native_tools: provider_native_tools_config_for_preset(
            draft.native_tools_preset,
            &draft.native_tools_custom,
        ),
        definition: agena::provider::ConfiguredModelDefinition {
            lifecycle: parse_optional_model_lifecycle(draft.lifecycle.as_str())?,
            context_window_tokens: parse_optional_u32_field(
                draft.context_window_tokens.as_str(),
                "context_window_tokens",
            )?,
            max_input_tokens: parse_optional_u32_field(
                draft.max_input_tokens.as_str(),
                "max_input_tokens",
            )?,
            max_output_tokens: parse_optional_u32_field(
                draft.max_output_tokens.as_str(),
                "max_output_tokens",
            )?,
            display_name: trimmed_owned_local(draft.display_name.as_str()),
            description: trimmed_owned_local(draft.description.as_str()),
            knowledge_cutoff: None,
            release_date: None,
            last_updated: None,
            open_weights: None,
            default_thinking_mode: None,
            supports_parallel_tool_calls: None,
            supports_verbosity: None,
            default_verbosity: None,
            default_temperature: None,
            default_top_p: None,
            default_top_k: None,
            assistant_reasoning_interleaved: None,
            assistant_reasoning_field: None,
            output_modalities: split_csv_tokens(draft.output_modalities.as_str()),
            pricing: None,
            thinking_modes: std::collections::BTreeMap::new(),
            speed_modes: std::collections::BTreeMap::new(),
            capabilities: agena::provider::ModelCapabilityPatch { input, features },
        },
    };

    let value = provider_model_overlay_to_json_local(overlay)?;
    Ok((model_id.to_owned(), value))
}

pub(super) fn provider_model_config_field_label(
    i18n: &I18n,
    field: ProviderModelConfigField,
) -> String {
    ui_text::t(i18n, provider_model_config_field_label_key(field))
}

pub(super) fn provider_model_config_field_prompt(
    i18n: &I18n,
    field: ProviderModelConfigField,
) -> String {
    i18n.text_args(
        "overlay-provider-studio-model-field-prompt",
        &crate::fl_args!("field" => provider_model_config_field_label(i18n, field)),
    )
}

pub(super) fn provider_model_config_field_value(
    draft: &ProviderModelConfigDraft,
    field: ProviderModelConfigField,
) -> String {
    match field {
        ProviderModelConfigField::ModelId => draft.model_id.clone(),
        ProviderModelConfigField::Enabled => draft.enabled.to_string(),
        ProviderModelConfigField::DisplayName => draft.display_name.clone(),
        ProviderModelConfigField::Lifecycle => draft.lifecycle.clone(),
        ProviderModelConfigField::ContextWindowTokens => draft.context_window_tokens.clone(),
        ProviderModelConfigField::MaxInputTokens => draft.max_input_tokens.clone(),
        ProviderModelConfigField::MaxOutputTokens => draft.max_output_tokens.clone(),
        ProviderModelConfigField::InputModalities => draft
            .input_modalities
            .iter()
            .cloned()
            .collect::<Vec<_>>()
            .join(", "),
        ProviderModelConfigField::Features => draft
            .features
            .iter()
            .cloned()
            .collect::<Vec<_>>()
            .join(", "),
        ProviderModelConfigField::OutputModalities => draft.output_modalities.clone(),
        ProviderModelConfigField::Description => draft.description.clone(),
        ProviderModelConfigField::NativeTools => draft.native_tools_preset.token().to_owned(),
    }
}

pub(super) fn provider_model_config_field_display(
    i18n: &I18n,
    draft: &ProviderModelConfigDraft,
    field: ProviderModelConfigField,
) -> String {
    let value = provider_model_config_field_value(draft, field);
    if value.trim().is_empty() {
        ui_text::t(i18n, "value-unset")
    } else if field == ProviderModelConfigField::NativeTools {
        provider_native_tools_preset_label(i18n, draft.native_tools_preset)
    } else {
        value
    }
}

pub(super) fn provider_model_config_field_editable(field: ProviderModelConfigField) -> bool {
    field != ProviderModelConfigField::ModelId
}

pub(super) fn commit_provider_model_config_field(
    draft: &mut ProviderModelConfigDraft,
    field: ProviderModelConfigField,
    value: String,
) -> std::result::Result<(), String> {
    match field {
        ProviderModelConfigField::ModelId => {
            let value = value.trim();
            if value.is_empty() {
                return Err("model id is required".to_owned());
            }
            draft.model_id = value.to_owned();
        }
        ProviderModelConfigField::Enabled => {
            draft.enabled = parse_bool_token(value.as_str())?;
        }
        ProviderModelConfigField::DisplayName => draft.display_name = value,
        ProviderModelConfigField::Lifecycle => {
            parse_optional_model_lifecycle(value.as_str())?;
            draft.lifecycle = value.trim().to_owned();
        }
        ProviderModelConfigField::ContextWindowTokens => {
            parse_optional_u32_field(value.as_str(), "context_window_tokens")?;
            draft.context_window_tokens = value.trim().to_owned();
        }
        ProviderModelConfigField::MaxInputTokens => {
            parse_optional_u32_field(value.as_str(), "max_input_tokens")?;
            draft.max_input_tokens = value.trim().to_owned();
        }
        ProviderModelConfigField::MaxOutputTokens => {
            parse_optional_u32_field(value.as_str(), "max_output_tokens")?;
            draft.max_output_tokens = value.trim().to_owned();
        }
        ProviderModelConfigField::InputModalities => {
            draft.input_modalities = parse_model_input_modality_set(value.as_str())?;
        }
        ProviderModelConfigField::Features => {
            draft.features = parse_model_capability_feature_set(value.as_str())?;
        }
        ProviderModelConfigField::OutputModalities => {
            draft.output_modalities = split_csv_tokens(value.as_str()).join(", ");
        }
        ProviderModelConfigField::Description => draft.description = value,
        ProviderModelConfigField::NativeTools => {
            let preset = if value.trim().is_empty() {
                ProviderNativeToolsPreset::Disabled
            } else {
                ProviderNativeToolsPreset::parse(value.as_str())
                    .ok_or_else(|| format!("unsupported native tools preset `{value}`"))?
            };
            draft.native_tools_preset = preset;
        }
    }
    Ok(())
}

fn provider_model_config_field_label_key(field: ProviderModelConfigField) -> &'static str {
    match field {
        ProviderModelConfigField::ModelId => "provider-model-field-model-id",
        ProviderModelConfigField::Enabled => "provider-model-field-enabled",
        ProviderModelConfigField::DisplayName => "provider-model-field-display-name",
        ProviderModelConfigField::Lifecycle => "provider-model-field-lifecycle",
        ProviderModelConfigField::ContextWindowTokens => "provider-model-field-context-window",
        ProviderModelConfigField::MaxInputTokens => "provider-model-field-max-input",
        ProviderModelConfigField::MaxOutputTokens => "provider-model-field-max-output",
        ProviderModelConfigField::InputModalities => "provider-model-field-input-modalities",
        ProviderModelConfigField::Features => "provider-model-field-features",
        ProviderModelConfigField::OutputModalities => "provider-model-field-output-modalities",
        ProviderModelConfigField::Description => "provider-model-field-description",
        ProviderModelConfigField::NativeTools => "provider-model-field-native-tools",
    }
}

pub(super) fn provider_native_tools_available_preset_for_adapter(
    adapter_id: &str,
) -> Option<ProviderNativeToolsPreset> {
    match adapter_id.trim() {
        "openai" => Some(ProviderNativeToolsPreset::OpenAiHostedDefaults),
        "anthropic" => Some(ProviderNativeToolsPreset::AnthropicHostedDefaults),
        "gemini" => Some(ProviderNativeToolsPreset::GeminiHostedDefaults),
        _ => None,
    }
}

pub(super) fn provider_native_tools_preset_label(
    i18n: &I18n,
    preset: ProviderNativeToolsPreset,
) -> String {
    match preset {
        ProviderNativeToolsPreset::Disabled => {
            ui_text::t(i18n, "provider-native-tools-disabled-label")
        }
        ProviderNativeToolsPreset::OpenAiHostedDefaults => {
            ui_text::t(i18n, "provider-native-tools-openai-label")
        }
        ProviderNativeToolsPreset::AnthropicHostedDefaults => {
            ui_text::t(i18n, "provider-native-tools-anthropic-label")
        }
        ProviderNativeToolsPreset::GeminiHostedDefaults => {
            ui_text::t(i18n, "provider-native-tools-gemini-label")
        }
        ProviderNativeToolsPreset::Custom => ui_text::t(i18n, "provider-native-tools-custom-label"),
    }
}

fn provider_model_overlay_to_json_local(
    overlay: agena::config::ProviderModelOverlay,
) -> std::result::Result<JsonValue, String> {
    if overlay.enabled && overlay.native_tools.is_empty() && overlay.definition.is_empty() {
        return Ok(JsonValue::Object(JsonMap::new()));
    }
    match serde_json::to_value(overlay).map_err(|error| error.to_string())? {
        JsonValue::Object(mut object) => {
            if matches!(object.get("enabled"), Some(JsonValue::Bool(true))) {
                object.remove("enabled");
            }
            Ok(JsonValue::Object(object))
        }
        other => Ok(other),
    }
}

fn trimmed_owned_local(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

fn parse_optional_u32_field(
    value: &str,
    field: &'static str,
) -> std::result::Result<Option<u32>, String> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    value
        .parse::<u32>()
        .map(Some)
        .map_err(|_| format!("{field} must be an unsigned integer"))
}

fn parse_optional_model_lifecycle(
    value: &str,
) -> std::result::Result<Option<agena::model::ModelLifecycle>, String> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    serde_json::from_value::<agena::model::ModelLifecycle>(JsonValue::String(value.to_owned()))
        .map(Some)
        .map_err(|_| format!("unsupported lifecycle `{value}`"))
}

fn model_lifecycle_token(value: agena::model::ModelLifecycle) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_default()
}

fn split_csv_tokens(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .collect()
}

fn parse_bool_token(value: &str) -> std::result::Result<bool, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" | "yes" | "1" | "enabled" => Ok(true),
        "false" | "no" | "0" | "disabled" => Ok(false),
        other => Err(format!("unsupported boolean `{other}`")),
    }
}

fn parse_model_input_modality(value: &str) -> Option<agena::model::ModelInputModality> {
    match value.trim() {
        "text" => Some(agena::model::ModelInputModality::Text),
        "image" => Some(agena::model::ModelInputModality::Image),
        "document" => Some(agena::model::ModelInputModality::Document),
        "audio" => Some(agena::model::ModelInputModality::Audio),
        "video" => Some(agena::model::ModelInputModality::Video),
        "file" => Some(agena::model::ModelInputModality::File),
        _ => None,
    }
}

fn parse_model_input_modality_set(value: &str) -> std::result::Result<BTreeSet<String>, String> {
    let mut parsed = BTreeSet::new();
    for token in split_csv_tokens(value) {
        if parse_model_input_modality(token.as_str()).is_none() {
            return Err(format!("unsupported input modality `{token}`"));
        }
        parsed.insert(token);
    }
    Ok(parsed)
}

fn parse_model_capability_feature(value: &str) -> Option<agena::provider::ModelCapabilityFeature> {
    match value.trim() {
        "tool_calling" => Some(agena::provider::ModelCapabilityFeature::ToolCalling),
        "streaming" => Some(agena::provider::ModelCapabilityFeature::Streaming),
        "reasoning" => Some(agena::provider::ModelCapabilityFeature::Reasoning),
        "structured_output" => Some(agena::provider::ModelCapabilityFeature::StructuredOutput),
        "temperature" => Some(agena::provider::ModelCapabilityFeature::Temperature),
        _ => None,
    }
}

fn parse_model_capability_feature_set(
    value: &str,
) -> std::result::Result<BTreeSet<String>, String> {
    let mut parsed = BTreeSet::new();
    for token in split_csv_tokens(value) {
        if parse_model_capability_feature(token.as_str()).is_none() {
            return Err(format!("unsupported model feature `{token}`"));
        }
        parsed.insert(token);
    }
    Ok(parsed)
}
