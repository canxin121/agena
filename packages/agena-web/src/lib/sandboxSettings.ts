import type { RuntimeSettingsLayer, RuntimeSettingsReadBundle } from './runtimeSettings'
import { hasRuntimeLayerValue, runtimeSettingLayer } from './runtimeSettingLayer'

export const SANDBOX_SETTING_KEYS = ['sandbox', 'resource_guard'] as const
export type SandboxSettingKey = (typeof SANDBOX_SETTING_KEYS)[number]
export type SandboxSettings = Record<SandboxSettingKey, boolean | undefined>
export type SandboxSettingSources = Record<SandboxSettingKey, RuntimeSettingsReadBundle>

function explicitBoolean(value: unknown): boolean | undefined {
  return typeof value === 'boolean' ? value : undefined
}

export function sandboxLayerOverride(
  sources: SandboxSettingSources | null,
  layer: RuntimeSettingsLayer,
  key: SandboxSettingKey,
): boolean | undefined {
  const response = sources?.[key]?.[layer]
  return hasRuntimeLayerValue(response) ? explicitBoolean(response?.value) : undefined
}

/** Edit the selected configuration layer even when a later layer overrides it. */
export function sandboxLayerSettings(
  sources: SandboxSettingSources | null,
  layer: RuntimeSettingsLayer,
): SandboxSettings {
  return Object.fromEntries(
    SANDBOX_SETTING_KEYS.map((key) => {
      const state = runtimeSettingLayer(sources?.[key] || null, layer, undefined)
      return [key, state.valueKnown ? explicitBoolean(state.value) : undefined]
    }),
  ) as SandboxSettings
}

export function sandboxInheritedValue(
  sources: SandboxSettingSources | null,
  layer: RuntimeSettingsLayer,
  key: SandboxSettingKey,
): boolean | undefined {
  const state = runtimeSettingLayer(sources?.[key] || null, layer, undefined)
  return state.inheritedKnown ? explicitBoolean(state.inherited) : undefined
}

export function sandboxInheritedKnown(
  sources: SandboxSettingSources | null,
  layer: RuntimeSettingsLayer,
  key: SandboxSettingKey,
): boolean {
  return runtimeSettingLayer(sources?.[key] || null, layer, undefined).inheritedKnown
}

export function sandboxEffectiveSettings(sources: SandboxSettingSources | null): SandboxSettings {
  const configured = sandboxLayerSettings(sources, 'workspace')
  return Object.fromEntries(
    SANDBOX_SETTING_KEYS.map((key) => [
      key,
      sources?.[key]?.effective.value === undefined ? configured[key] : explicitBoolean(sources[key].effective.value),
    ]),
  ) as SandboxSettings
}

export function sandboxExecutionState(sources: SandboxSettingSources | null) {
  const effective = sandboxEffectiveSettings(sources)
  return {
    sandbox: effective.sandbox,
    resourceGuard: effective.sandbox === undefined ? undefined : !effective.sandbox && effective.resource_guard,
    resourceGuardStandby: effective.sandbox === undefined ? undefined : effective.sandbox && effective.resource_guard,
    environmentDisabled: effective.sandbox === false && sandboxLayerSettings(sources, 'workspace').sandbox === true,
  }
}
