pub(in crate::app) fn composer_item_needs_summary_chip(item: &ComposerItem) -> bool {
    matches!(item, ComposerItem::Attachment(_))
}

pub(in crate::app) fn model_catalog_list_subtitle(
    i18n: &I18n,
    entry: &CatalogModelResource,
) -> String {
    let mut parts = Vec::new();
    if let Some(display_name) = entry
        .display_name
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        parts.push(display_name.to_owned());
    }
    if let Some(origin) = entry
        .origin
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        parts.push(origin.to_owned());
    }
    if let Some(lifecycle) = entry.lifecycle {
        parts.push(model_catalog_lifecycle_label(i18n, lifecycle));
    }
    if model_catalog_has_token_limits(entry) {
        parts.push(model_catalog_limits_summary(i18n, entry));
    }
    let feature_summary = model_catalog_supported_feature_summary(&entry.capabilities);
    if !feature_summary.is_empty() {
        parts.push(feature_summary);
    }
    let pricing = model_catalog_pricing_summary(i18n, entry.pricing.as_ref());
    if pricing != ui_text::t(i18n, "value-unset") {
        parts.push(pricing);
    }
    join_inline_segments(parts)
}

pub(in crate::app) fn model_catalog_detail_text(
    i18n: &I18n,
    entry: &CatalogModelResource,
) -> Text<'static> {
    let mut lines = vec![
        model_catalog_detail_labeled_line(
            i18n,
            "overlay-model-catalog-field-model-id",
            entry.model_id.as_str(),
        ),
        model_catalog_detail_labeled_line(
            i18n,
            "overlay-model-catalog-field-display",
            model_catalog_optional_string(i18n, entry.display_name.as_deref()),
        ),
        model_catalog_detail_labeled_line(
            i18n,
            "overlay-model-catalog-field-origin",
            model_catalog_optional_string(i18n, entry.origin.as_deref()),
        ),
        model_catalog_detail_labeled_line(
            i18n,
            "overlay-model-catalog-field-lifecycle",
            entry
                .lifecycle
                .map(|lifecycle| model_catalog_lifecycle_label(i18n, lifecycle))
                .unwrap_or_else(|| ui_text::t(i18n, "value-unset")),
        ),
        model_catalog_detail_labeled_line(
            i18n,
            "overlay-model-catalog-field-dates",
            model_catalog_dates_summary(i18n, entry),
        ),
        model_catalog_detail_labeled_line(
            i18n,
            "overlay-model-catalog-field-limits",
            model_catalog_limits_summary(i18n, entry),
        ),
        model_catalog_detail_labeled_line(
            i18n,
            "overlay-model-catalog-field-inputs",
            model_catalog_input_capability_summary(i18n, &entry.capabilities),
        ),
        model_catalog_detail_labeled_line(
            i18n,
            "overlay-model-catalog-field-output",
            model_catalog_string_list_summary(i18n, &entry.output_modalities),
        ),
        model_catalog_detail_labeled_line(
            i18n,
            "overlay-model-catalog-field-features",
            model_catalog_feature_capability_summary(i18n, &entry.capabilities),
        ),
        model_catalog_detail_labeled_line(
            i18n,
            "overlay-model-catalog-field-modes",
            model_catalog_modes_summary(i18n, entry),
        ),
        model_catalog_detail_labeled_line(
            i18n,
            "overlay-model-catalog-field-defaults",
            model_catalog_defaults_summary(i18n, entry),
        ),
        model_catalog_detail_labeled_line(
            i18n,
            "overlay-model-catalog-field-runtime",
            model_catalog_runtime_summary(i18n, entry),
        ),
        model_catalog_detail_labeled_line(
            i18n,
            "overlay-model-catalog-field-pricing",
            model_catalog_pricing_summary(i18n, entry.pricing.as_ref()),
        ),
        model_catalog_detail_labeled_line(
            i18n,
            "overlay-model-catalog-field-source",
            model_catalog_source_summary(entry),
        ),
    ];
    if let Some(description) = entry.description.as_deref()
        && !description.trim().is_empty()
    {
        lines.push(DetailTextLine::plain(String::new(), Style::default()));
        lines.push(DetailTextLine::plain(
            sanitize_display_text(description),
            Style::default(),
        ));
    }
    build_detail_text(lines, &DetailTextSpec::label_width(14))
}

pub(in crate::app) fn model_catalog_detail_labeled_line(
    i18n: &I18n,
    label_key: &str,
    value: impl Into<String>,
) -> DetailTextLine<'static> {
    let value = value.into();
    DetailTextLine::labeled(
        ui_text::t(i18n, label_key),
        sanitize_display_text(value.as_str()),
        Style::default().fg(agena_tui_components::theme::muted_color()),
        Style::default(),
    )
}

pub(in crate::app) fn model_catalog_optional_string(i18n: &I18n, value: Option<&str>) -> String {
    value
        .filter(|value| !value.trim().is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| ui_text::t(i18n, "value-unset"))
}

pub(in crate::app) fn model_catalog_lifecycle_label(
    i18n: &I18n,
    value: agena::model::ModelLifecycle,
) -> String {
    let key = match value {
        agena::model::ModelLifecycle::Active => "overlay-model-catalog-lifecycle-active",
        agena::model::ModelLifecycle::Preview => "overlay-model-catalog-lifecycle-preview",
        agena::model::ModelLifecycle::Beta => "overlay-model-catalog-lifecycle-beta",
        agena::model::ModelLifecycle::Alpha => "overlay-model-catalog-lifecycle-alpha",
        agena::model::ModelLifecycle::Experimental => {
            "overlay-model-catalog-lifecycle-experimental"
        }
        agena::model::ModelLifecycle::Deprecated => "overlay-model-catalog-lifecycle-deprecated",
    };
    ui_text::t(i18n, key)
}

pub(in crate::app) fn model_catalog_dates_summary(
    i18n: &I18n,
    entry: &CatalogModelResource,
) -> String {
    let mut parts = Vec::new();
    if let Some(value) = entry
        .release_date
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        parts.push(i18n.text_args(
            "overlay-model-catalog-date-release",
            &crate::fl_args!("value" => value),
        ));
    }
    if let Some(value) = entry
        .last_updated
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        parts.push(i18n.text_args(
            "overlay-model-catalog-date-updated",
            &crate::fl_args!("value" => value),
        ));
    }
    if let Some(value) = entry
        .knowledge_cutoff
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        parts.push(i18n.text_args(
            "overlay-model-catalog-date-cutoff",
            &crate::fl_args!("value" => value),
        ));
    }
    if parts.is_empty() {
        ui_text::t(i18n, "value-unset")
    } else {
        join_inline_segments(parts)
    }
}

pub(in crate::app) fn model_catalog_limits_summary(
    i18n: &I18n,
    entry: &CatalogModelResource,
) -> String {
    sanitize_display_text(i18n.text_args(
        "overlay-model-catalog-limits",
        &crate::fl_args!(
            "context" => model_catalog_token_count(i18n, entry.context_window_tokens),
            "input" => model_catalog_token_count(i18n, entry.max_input_tokens),
            "output" => model_catalog_token_count(i18n, entry.max_output_tokens),
        ),
    ))
}

pub(in crate::app) fn model_catalog_has_token_limits(entry: &CatalogModelResource) -> bool {
    entry.context_window_tokens.is_some()
        || entry.max_input_tokens.is_some()
        || entry.max_output_tokens.is_some()
}

pub(in crate::app) fn model_catalog_token_count(i18n: &I18n, value: Option<u32>) -> String {
    value
        .map(|value| format_tokens_k(value as u64))
        .unwrap_or_else(|| ui_text::t(i18n, "value-unset"))
}

pub(in crate::app) fn model_catalog_input_capability_summary(
    i18n: &I18n,
    capabilities: &agena::provider::ModelCapabilityPatch,
) -> String {
    let mut supported = Vec::new();
    let mut unsupported = Vec::new();
    for modality in [
        agena::model::ModelInputModality::Text,
        agena::model::ModelInputModality::Image,
        agena::model::ModelInputModality::Document,
        agena::model::ModelInputModality::Audio,
        agena::model::ModelInputModality::Video,
        agena::model::ModelInputModality::File,
    ] {
        match capabilities.input_support(modality) {
            Some(agena::model::CapabilitySupport::Supported) => {
                supported.push(modality.to_string())
            }
            Some(agena::model::CapabilitySupport::Unsupported) => {
                unsupported.push(modality.to_string())
            }
            Some(agena::model::CapabilitySupport::Unknown) | None => {}
        }
    }
    model_catalog_support_summary(i18n, supported, unsupported)
}

pub(in crate::app) fn model_catalog_feature_capability_summary(
    i18n: &I18n,
    capabilities: &agena::provider::ModelCapabilityPatch,
) -> String {
    let mut supported = Vec::new();
    let mut unsupported = Vec::new();
    for feature in [
        agena::provider::ModelCapabilityFeature::ToolCalling,
        agena::provider::ModelCapabilityFeature::Streaming,
        agena::provider::ModelCapabilityFeature::Reasoning,
        agena::provider::ModelCapabilityFeature::StructuredOutput,
        agena::provider::ModelCapabilityFeature::Temperature,
    ] {
        match capabilities.feature_support(feature) {
            Some(agena::model::CapabilitySupport::Supported) => {
                supported.push(model_catalog_feature_label(feature).to_owned())
            }
            Some(agena::model::CapabilitySupport::Unsupported) => {
                unsupported.push(model_catalog_feature_label(feature).to_owned())
            }
            Some(agena::model::CapabilitySupport::Unknown) | None => {}
        }
    }
    model_catalog_support_summary(i18n, supported, unsupported)
}

pub(in crate::app) fn model_catalog_supported_feature_summary(
    capabilities: &agena::provider::ModelCapabilityPatch,
) -> String {
    [
        agena::provider::ModelCapabilityFeature::ToolCalling,
        agena::provider::ModelCapabilityFeature::Streaming,
        agena::provider::ModelCapabilityFeature::Reasoning,
        agena::provider::ModelCapabilityFeature::StructuredOutput,
        agena::provider::ModelCapabilityFeature::Temperature,
    ]
    .into_iter()
    .filter(|feature| {
        matches!(
            capabilities.feature_support(*feature),
            Some(agena::model::CapabilitySupport::Supported)
        )
    })
    .map(model_catalog_feature_label)
    .collect::<Vec<_>>()
    .join(", ")
}

pub(in crate::app) fn model_catalog_feature_label(
    feature: agena::provider::ModelCapabilityFeature,
) -> &'static str {
    match feature {
        agena::provider::ModelCapabilityFeature::ToolCalling => "tools",
        agena::provider::ModelCapabilityFeature::Streaming => "stream",
        agena::provider::ModelCapabilityFeature::Reasoning => "reasoning",
        agena::provider::ModelCapabilityFeature::StructuredOutput => "structured",
        agena::provider::ModelCapabilityFeature::Temperature => "temperature",
    }
}

pub(in crate::app) fn model_catalog_support_summary(
    i18n: &I18n,
    supported: Vec<String>,
    unsupported: Vec<String>,
) -> String {
    let mut parts = Vec::new();
    if !supported.is_empty() {
        parts.push(format_key_value_segment("+", supported.join(", ").as_str()));
    }
    if !unsupported.is_empty() {
        parts.push(format_key_value_segment(
            "-",
            unsupported.join(", ").as_str(),
        ));
    }
    if parts.is_empty() {
        ui_text::t(i18n, "value-unset")
    } else {
        join_inline_segments(parts)
    }
}

pub(in crate::app) fn model_catalog_string_list_summary(i18n: &I18n, values: &[String]) -> String {
    if values.is_empty() {
        ui_text::t(i18n, "value-unset")
    } else {
        values.join(", ")
    }
}

pub(in crate::app) fn model_catalog_modes_summary(
    i18n: &I18n,
    entry: &CatalogModelResource,
) -> String {
    let mut parts = Vec::new();
    if let Some(default) = entry.thinking_modes.default.mode() {
        parts.push(format_key_value_segment(
            ui_text::t(i18n, "overlay-model-catalog-default-thinking").as_str(),
            default,
        ));
    }
    if let Some(default) = entry.speed_modes.default.mode() {
        parts.push(format_key_value_segment(
            ui_text::t(i18n, "overlay-model-catalog-default-speed").as_str(),
            default,
        ));
    }
    let thinking_modes = model_catalog_thinking_mode_names(entry);
    if !thinking_modes.is_empty() {
        parts.push(format_key_value_segment(
            ui_text::t(i18n, "overlay-model-catalog-thinking-modes").as_str(),
            thinking_modes.as_str(),
        ));
    }
    let speed_modes = model_catalog_speed_mode_names(entry);
    if !speed_modes.is_empty() {
        parts.push(format_key_value_segment(
            ui_text::t(i18n, "overlay-model-catalog-speed-modes").as_str(),
            speed_modes.as_str(),
        ));
    }
    if parts.is_empty() {
        ui_text::t(i18n, "value-unset")
    } else {
        join_inline_segments(parts)
    }
}

pub(in crate::app) fn model_catalog_thinking_mode_names(entry: &CatalogModelResource) -> String {
    let mut modes = entry
        .thinking_modes
        .iter()
        .filter(|(_, mode)| !mode.disabled)
        .collect::<Vec<_>>();
    modes.sort_by(|left, right| {
        agena::model::compare_thinking_mode_strength(
            &agena::provider::configured_thinking_mode_to_model(left.0, left.1),
            &agena::provider::configured_thinking_mode_to_model(right.0, right.1),
        )
    });
    modes
        .into_iter()
        .map(|(selector, mode)| {
            mode.display_name
                .as_deref()
                .filter(|value| !value.trim().is_empty())
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| ui_text::thinking_mode_display_value(selector.as_str()))
        })
        .collect::<Vec<_>>()
        .join(", ")
}

pub(in crate::app) fn model_catalog_speed_mode_names(entry: &CatalogModelResource) -> String {
    entry
        .speed_modes
        .iter()
        .filter(|(_, mode)| !mode.disabled)
        .map(|(name, mode)| {
            if let Some(display_name) = mode
                .display_name
                .as_deref()
                .filter(|value| !value.trim().is_empty())
            {
                display_name.to_owned()
            } else {
                ui_text::speed_mode_display_value(name)
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

pub(in crate::app) fn model_catalog_defaults_summary(
    i18n: &I18n,
    entry: &CatalogModelResource,
) -> String {
    let mut parts = Vec::new();
    if let Some(value) = entry
        .default_verbosity
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        parts.push(format_key_value_segment(
            ui_text::t(i18n, "overlay-model-catalog-default-verbosity").as_str(),
            value,
        ));
    }
    if let Some(value) = entry
        .default_temperature
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        parts.push(format_key_value_segment(
            ui_text::t(i18n, "overlay-model-catalog-default-temperature").as_str(),
            value,
        ));
    }
    if let Some(value) = entry
        .default_top_p
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        parts.push(format_key_value_segment(
            ui_text::t(i18n, "overlay-model-catalog-default-top-p").as_str(),
            value,
        ));
    }
    if let Some(value) = entry.default_top_k {
        parts.push(format_key_value_segment(
            ui_text::t(i18n, "overlay-model-catalog-default-top-k").as_str(),
            value.to_string().as_str(),
        ));
    }
    if parts.is_empty() {
        ui_text::t(i18n, "value-unset")
    } else {
        join_inline_segments(parts)
    }
}

pub(in crate::app) fn model_catalog_runtime_summary(
    i18n: &I18n,
    entry: &CatalogModelResource,
) -> String {
    let mut parts = Vec::new();
    if let Some(value) = entry.supports_parallel_tool_calls {
        parts.push(format_key_value_segment(
            ui_text::t(i18n, "overlay-model-catalog-parallel-tools").as_str(),
            model_catalog_bool_label(i18n, value).as_str(),
        ));
    }
    if let Some(value) = entry.supports_verbosity {
        parts.push(format_key_value_segment(
            ui_text::t(i18n, "overlay-model-catalog-supports-verbosity").as_str(),
            model_catalog_bool_label(i18n, value).as_str(),
        ));
    }
    if let Some(value) = entry.assistant_reasoning_interleaved {
        parts.push(format_key_value_segment(
            ui_text::t(i18n, "overlay-model-catalog-reasoning-interleaved").as_str(),
            model_catalog_bool_label(i18n, value).as_str(),
        ));
    }
    if let Some(value) = entry
        .assistant_reasoning_field
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        parts.push(format_key_value_segment(
            ui_text::t(i18n, "overlay-model-catalog-reasoning-field").as_str(),
            value,
        ));
    }
    if let Some(value) = entry.open_weights {
        parts.push(format_key_value_segment(
            ui_text::t(i18n, "overlay-model-catalog-open-weights").as_str(),
            model_catalog_bool_label(i18n, value).as_str(),
        ));
    }
    if parts.is_empty() {
        ui_text::t(i18n, "value-unset")
    } else {
        join_inline_segments(parts)
    }
}

pub(in crate::app) fn model_catalog_bool_label(i18n: &I18n, value: bool) -> String {
    ui_text::t(i18n, if value { "value-yes" } else { "value-no" })
}

pub(in crate::app) fn model_catalog_pricing_summary(
    i18n: &I18n,
    pricing: Option<&agena::model::ModelPricing>,
) -> String {
    let Some(pricing) = pricing.filter(|pricing| !pricing.is_empty()) else {
        return ui_text::t(i18n, "value-unset");
    };
    let mut parts = Vec::new();
    if let Some(value) = pricing.input_usd_per_million_tokens.as_deref() {
        parts.push(sanitize_display_text(i18n.text_args(
            "overlay-model-catalog-price-input",
            &crate::fl_args!("value" => value),
        )));
    }
    if let Some(value) = pricing.output_usd_per_million_tokens.as_deref() {
        parts.push(sanitize_display_text(i18n.text_args(
            "overlay-model-catalog-price-output",
            &crate::fl_args!("value" => value),
        )));
    }
    if let Some(value) = pricing.cache_read_usd_per_million_tokens.as_deref() {
        parts.push(sanitize_display_text(i18n.text_args(
            "overlay-model-catalog-price-cache-read",
            &crate::fl_args!("value" => value),
        )));
    }
    if let Some(value) = pricing.cache_write_usd_per_million_tokens.as_deref() {
        parts.push(sanitize_display_text(i18n.text_args(
            "overlay-model-catalog-price-cache-write",
            &crate::fl_args!("value" => value),
        )));
    }
    if !pricing.tiers.is_empty() {
        parts.push(sanitize_display_text(i18n.text_args(
            "overlay-model-catalog-tier-count",
            &crate::fl_args!("count" => pricing.tiers.len() as i64),
        )));
    }
    if parts.is_empty() {
        ui_text::t(i18n, "value-unset")
    } else {
        join_inline_segments(parts)
    }
}

pub(in crate::app) fn model_catalog_source_summary(entry: &CatalogModelResource) -> String {
    entry
        .source_label
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| format!("{:?}", entry.source).to_ascii_lowercase())
}
use super::{
    CatalogModelResource, ComposerItem, DetailTextLine, DetailTextSpec, I18n, Style, Text,
    build_detail_text, format_key_value_segment, format_tokens_k, join_inline_segments,
    sanitize_display_text, ui_text,
};
