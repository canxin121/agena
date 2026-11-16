import type { RuntimeSettingsLayer, RuntimeSettingsReadBundle } from './runtimeSettings'

export const SANDBOX_SETTING_KEYS = ['sandbox', 'resource_guard'] as const
export type SandboxSettingKey = (typeof SANDBOX_SETTING_KEYS)[number]
export type SandboxSettings = Record<SandboxSettingKey, boolean>
export type SandboxSettingSources = Record<SandboxSettingKey, RuntimeSettingsReadBundle>

function explicitBoolean(value: unknown): boolean | undefined {
  return typeof value === 'boolean' ? value : undefined
}

export function sandboxLayerOverride(
  sources: SandboxSettingSources | null,
  layer: RuntimeSettingsLayer,
  key: SandboxSettingKey,
): boolean | undefined {
  return explicitBoolean(sources?.[key]?.[layer]?.value)
}

/** Edit the selected configuration layer even when a later layer overrides it. */
export function sandboxLayerSettings(
  sources: SandboxSettingSources | null,
  layer: RuntimeSettingsLayer,
): SandboxSettings {
  return Object.fromEntries(
    SANDBOX_SETTING_KEYS.map((key) => [
      key,
      sandboxLayerOverride(sources, layer, key) ??
        (layer === 'workspace' ? sandboxLayerOverride(sources, 'global', key) : undefined) ??
        true,
    ]),
  ) as SandboxSettings
}

export function sandboxEffectiveSettings(sources: SandboxSettingSources | null): SandboxSettings {
  const configured = sandboxLayerSettings(sources, 'workspace')
  return Object.fromEntries(
    SANDBOX_SETTING_KEYS.map((key) => [key, explicitBoolean(sources?.[key]?.effective.value) ?? configured[key]]),
  ) as SandboxSettings
}

export function sandboxExecutionState(sources: SandboxSettingSources | null) {
  const effective = sandboxEffectiveSettings(sources)
  return {
    sandbox: effective.sandbox,
    resourceGuard: !effective.sandbox && effective.resource_guard,
    resourceGuardStandby: effective.sandbox && effective.resource_guard,
    environmentDisabled: !effective.sandbox && sandboxLayerSettings(sources, 'workspace').sandbox,
  }
}
