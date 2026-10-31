use std::collections::BTreeMap;

use crate::catalog_merge::{
    merge_catalog_mode_groups, merge_catalog_speed_mode_fill_missing, merge_catalog_thinking_mode,
    merge_provider_sources, merge_source_priority,
};
use crate::{
    CatalogModelDefinition, ModelCatalogDocument, merge_capability_patch, merge_model_pricing,
    merge_unique,
};

pub fn merge_public_source_catalog_document(
    current: &mut BTreeMap<String, CatalogModelDefinition>,
    next: ModelCatalogDocument,
) {
    for (model_id, definition) in next.models {
        current
            .entry(model_id)
            .and_modify(|existing| merge_public_source_catalog_definition(existing, &definition))
            .or_insert(definition);
    }
}

pub fn merge_public_source_catalog_definition(
    current: &mut CatalogModelDefinition,
    next: &CatalogModelDefinition,
) {
    let current_semantics_priority = current.source_priority.semantics_priority;
    let next_semantics_priority = next.source_priority.semantics_priority;
    if current.lifecycle.is_none() {
        current.lifecycle = next.lifecycle;
    }
    merge_limit_field(
        &mut current.context_window_tokens,
        next.context_window_tokens,
        current.source_priority.limits_priority,
        next.source_priority.limits_priority,
    );
    merge_limit_field(
        &mut current.max_input_tokens,
        next.max_input_tokens,
        current.source_priority.limits_priority,
        next.source_priority.limits_priority,
    );
    merge_limit_field(
        &mut current.max_output_tokens,
        next.max_output_tokens,
        current.source_priority.limits_priority,
        next.source_priority.limits_priority,
    );
    if current.description.is_none() {
        current.description = next.description.clone();
    }
    if current.knowledge_cutoff.is_none() {
        current.knowledge_cutoff = next.knowledge_cutoff.clone();
    }
    if current.release_date.is_none() {
        current.release_date = next.release_date.clone();
    }
    if current.last_updated.is_none() {
        current.last_updated = next.last_updated.clone();
    }
    if current.open_weights.is_none() {
        current.open_weights = next.open_weights;
    }
    if current.supports_parallel_tool_calls.is_none() {
        current.supports_parallel_tool_calls = next.supports_parallel_tool_calls;
    }
    merge_provider_sources(&mut current.provider_sources, &next.provider_sources);
    merge_verbosity_profiles_by_priority(
        current,
        next,
        current_semantics_priority,
        next_semantics_priority,
    );
    if current.supports_verbosity.is_none() {
        current.supports_verbosity = next.supports_verbosity;
    }
    if current.default_verbosity.is_none() {
        current.default_verbosity = next.default_verbosity.clone();
    }
    if current.default_temperature.is_none() {
        current.default_temperature = next.default_temperature.clone();
    }
    if current.default_top_p.is_none() {
        current.default_top_p = next.default_top_p.clone();
    }
    if current.default_top_k.is_none() {
        current.default_top_k = next.default_top_k;
    }
    if current.assistant_reasoning_interleaved.is_none() {
        current.assistant_reasoning_interleaved = next.assistant_reasoning_interleaved;
    }
    if current.assistant_reasoning_field.is_none() {
        current.assistant_reasoning_field = next.assistant_reasoning_field.clone();
    }
    merge_unique(&mut current.output_modalities, &next.output_modalities);
    merge_model_pricing(&mut current.pricing, next.pricing.as_ref());
    if current.display_name.is_none() {
        current.display_name = next.display_name.clone();
    }
    if current.origin.is_none() {
        current.origin = next.origin.clone();
    }
    merge_catalog_mode_groups(
        &mut current.thinking_modes,
        &next.thinking_modes,
        merge_catalog_thinking_mode,
    );
    merge_catalog_mode_groups(
        &mut current.speed_modes,
        &next.speed_modes,
        merge_catalog_speed_mode_fill_missing,
    );
    merge_capability_patch(&mut current.capabilities, &next.capabilities);
    merge_source_priority(&mut current.source_priority, &next.source_priority);
}

fn merge_verbosity_profiles_by_priority(
    current: &mut CatalogModelDefinition,
    next: &CatalogModelDefinition,
    current_priority: i32,
    next_priority: i32,
) {
    for (provider, profile) in &next.verbosity_by_provider {
        let should_replace = current
            .verbosity_by_provider
            .get(provider)
            .is_none_or(|_| next_priority > current_priority);
        if should_replace {
            current
                .verbosity_by_provider
                .insert(provider.clone(), profile.clone());
        }
    }
}

fn merge_limit_field(
    current: &mut Option<u32>,
    next: Option<u32>,
    current_priority: i32,
    next_priority: i32,
) {
    if current.is_none() || (next.is_some() && next_priority > current_priority) {
        *current = next.or(*current);
    }
}
