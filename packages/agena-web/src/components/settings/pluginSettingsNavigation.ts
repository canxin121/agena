export const PLUGIN_SETTINGS_VIEWS = {
  'plugin-workbench': 'overview',
  'plugin-settings': 'settings',
  'plugin-commands': 'commands',
  'plugin-tools': 'tools',
  'plugin-logs': 'logs',
  'plugin-diagnostics': 'diagnostics',
} as const

export type PluginSettingsTab = (typeof PLUGIN_SETTINGS_VIEWS)[keyof typeof PLUGIN_SETTINGS_VIEWS]

export function pluginTabForSettingsView(view: unknown, requestedTab?: unknown): PluginSettingsTab | null {
  const id = String(view || '')
    .trim()
    .toLowerCase()
  if (id === 'plugin-workbench') {
    const tab = String(requestedTab || '')
      .trim()
      .toLowerCase()
    return Object.values(PLUGIN_SETTINGS_VIEWS).find((value) => value === tab) || 'overview'
  }
  return Object.hasOwn(PLUGIN_SETTINGS_VIEWS, id)
    ? PLUGIN_SETTINGS_VIEWS[id as keyof typeof PLUGIN_SETTINGS_VIEWS]
    : null
}

/** Old sidebar destinations now open a tab inside the same plugin workbench. */
export function normalizePluginSettingsView(view: unknown): string {
  const id = String(view || '')
    .trim()
    .toLowerCase()
  return pluginTabForSettingsView(id) ? 'plugin-workbench' : id
}
