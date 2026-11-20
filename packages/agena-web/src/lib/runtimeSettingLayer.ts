import type { RuntimeSettingReadResponse, RuntimeSettingsLayer, RuntimeSettingsReadBundle } from './runtimeSettings'
import type { JsonValue } from '@/types/json'

/** Use schema-aware server presence; legacy optional fields treat null as absent. */
export function hasRuntimeLayerValue(response: RuntimeSettingReadResponse | null | undefined): boolean {
  if (typeof response?.has_override === 'boolean') return response.has_override
  return response?.config_found !== false && response?.value !== undefined && response?.value !== null
}

/** Defaults fill the global layer; only lower layers have a parent. */
export function runtimeSettingBaseValue(
  response: RuntimeSettingReadResponse | null | undefined,
  layer: RuntimeSettingsLayer,
): JsonValue {
  if (layer === 'workspace') return response?.inherited_value
  if (response?.default_value !== undefined) return response.default_value
  // Older servers used inherited_value for global defaults. Never use this
  // alias when the response already advertises the current layer contract.
  return response?.layer_value === undefined ? response?.inherited_value : undefined
}

/**
 * Project the layer being edited, before any workspace or environment overlay.
 * Server inheritance metadata is authoritative. Fallbacks keep scalar editors
 * usable, but cannot prove a parent value when its resolution was unavailable.
 */
export function runtimeSettingLayer(
  sources: RuntimeSettingsReadBundle | null,
  layer: RuntimeSettingsLayer,
  fallback: JsonValue,
) {
  const selected = sources?.[layer]
  const hasOverride = hasRuntimeLayerValue(selected)
  const baseValue = runtimeSettingBaseValue(selected, layer)
  const hasInheritedMetadata = baseValue !== undefined
  const hasResolvedMetadata = selected?.layer_value !== undefined
  const hasGlobalParent = layer === 'workspace' && hasRuntimeLayerValue(sources?.global)
  const inherited = hasInheritedMetadata ? baseValue : hasGlobalParent ? sources!.global.value : fallback
  // Groups have schema-specific merge rules; use the server projection instead
  // of attempting a generic deep merge or substituting the final effective value.
  const value = hasResolvedMetadata ? selected.layer_value : hasOverride ? selected?.value : inherited
  const effective = sources?.effective.value
  return {
    hasOverride,
    valueKnown: hasResolvedMetadata || hasOverride || hasInheritedMetadata,
    inheritedKnown: hasInheritedMetadata,
    inherited,
    value,
    effective,
    inheritedFrom: layer === 'workspace' ? 'global' : 'default',
    effectiveDiffers: effective !== undefined && JSON.stringify(effective) !== JSON.stringify(value),
  }
}
