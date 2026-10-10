import type { LocationQuery, LocationQueryRaw } from 'vue-router'
import type { SettingsSidebarDestination, SettingsTab } from './sidebar/settingsSidebarNavigation'
import { pluginTabForSettingsView } from './pluginSettingsNavigation'

export function settingsQueryForDestination(
  destination: SettingsSidebarDestination,
  currentSection: SettingsTab,
  currentQuery: LocationQuery,
): LocationQueryRaw {
  const { view: _view, plugin, pluginTab: _pluginTab, ...scopeQuery } = currentQuery
  const query: LocationQueryRaw = { ...scopeQuery, view: destination.view }
  if (
    destination.section === 'plugins-tools' &&
    pluginTabForSettingsView(destination.view) &&
    currentSection === 'plugins-tools' &&
    pluginTabForSettingsView(currentQuery.view)
  ) {
    if (plugin !== undefined) query.plugin = plugin
  }
  return query
}
