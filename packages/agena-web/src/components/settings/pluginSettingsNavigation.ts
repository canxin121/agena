export const PLUGIN_SETTINGS_TABS = [
  'overview',
  'settings',
  'commands',
  'tools',
  'hooks',
  'logs',
  'diagnostics',
] as const

export type PluginSettingsTab = (typeof PLUGIN_SETTINGS_TABS)[number]

export function pluginTabForSettingsView(view: unknown, requestedTab?: unknown): PluginSettingsTab | null {
  if (
    String(view || '')
      .trim()
      .toLowerCase() !== 'plugin-workbench'
  )
    return null
  const tab = String(requestedTab || '')
    .trim()
    .toLowerCase()
  return PLUGIN_SETTINGS_TABS.find((value) => value === tab) || 'overview'
}
