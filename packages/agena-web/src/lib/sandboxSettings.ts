import type { RuntimeSettingsLayer, RuntimeSettingsReadBundle } from './runtimeSettings'

export const SANDBOX_SETTING_KEYS = ['enabled', 'resource_checks', 'command_hints'] as const
export type SandboxSettingKey = (typeof SANDBOX_SETTING_KEYS)[number]
export type SandboxSettings = Record<SandboxSettingKey, boolean>

function explicitBoolean(value: unknown, key: SandboxSettingKey): boolean | undefined {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return undefined
  const field = (value as Record<string, unknown>)[key]
  return typeof field === 'boolean' ? field : undefined
}

export function sandboxLayerOverride(
  sources: RuntimeSettingsReadBundle | null,
  layer: RuntimeSettingsLayer,
  key: SandboxSettingKey,
): boolean | undefined {
  return explicitBoolean(sources?.[layer]?.value, key)
}

/** Edit the selected configuration layer even when a later layer overrides it. */
export function sandboxLayerSettings(
  sources: RuntimeSettingsReadBundle | null,
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

export function sandboxEffectiveSettings(sources: RuntimeSettingsReadBundle | null): SandboxSettings {
  const configured = sandboxLayerSettings(sources, 'workspace')
  return Object.fromEntries(
    SANDBOX_SETTING_KEYS.map((key) => [key, explicitBoolean(sources?.effective.value, key) ?? configured[key]]),
  ) as SandboxSettings
}

export function sandboxExecutionState(sources: RuntimeSettingsReadBundle | null) {
  const effective = sandboxEffectiveSettings(sources)
  const resourceChecks = !effective.enabled && effective.resource_checks
  return {
    enabled: effective.enabled,
    resourceChecks,
    commandHints: resourceChecks && effective.command_hints,
    environmentDisabled: !effective.enabled && sandboxLayerSettings(sources, 'workspace').enabled,
  }
}
