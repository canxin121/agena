export const PLUGIN_SETTINGS_VIEWS = {
  'plugin-workbench': 'overview',
  'plugin-settings': 'settings',
  'plugin-commands': 'commands',
  'plugin-tools': 'tools',
  'plugin-logs': 'logs',
  'plugin-diagnostics': 'diagnostics',
} as const

export type PluginSettingsTab = (typeof PLUGIN_SETTINGS_VIEWS)[keyof typeof PLUGIN_SETTINGS_VIEWS]

export function pluginTabForSettingsView(view: unknown): PluginSettingsTab | null {
  const id = String(view || '')
    .trim()
    .toLowerCase()
  return Object.hasOwn(PLUGIN_SETTINGS_VIEWS, id)
    ? PLUGIN_SETTINGS_VIEWS[id as keyof typeof PLUGIN_SETTINGS_VIEWS]
    : null
}

/** Keep existing plugin-workbench?pluginTab=... links usable. */
export function normalizePluginSettingsView(view: unknown, legacyTab?: unknown): string {
  const id = String(view || '')
    .trim()
    .toLowerCase()
  if (id !== 'plugin-workbench') return id
  const tab = String(legacyTab || '')
    .trim()
    .toLowerCase()
  return Object.entries(PLUGIN_SETTINGS_VIEWS).find(([, value]) => value === tab)?.[0] || id
}
