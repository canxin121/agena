import type { LocationQuery, LocationQueryRaw } from 'vue-router'
import type { SettingsSidebarDestination, SettingsTab } from './sidebar/settingsSidebarNavigation'
import { pluginTabForSettingsView } from './pluginSettingsNavigation'

export function settingsQueryForDestination(
  destination: SettingsSidebarDestination,
  currentSection: SettingsTab,
  currentQuery: LocationQuery,
): LocationQueryRaw {
  // `sessionId` is not a settings parameter: a session-scoped page edits the
  // conversation this client used last, so a stale id must not travel between
  // sections.
  const {
    view: _view,
    scope: _scope,
    sessionId: _sessionId,
    plugin,
    pluginTab: _pluginTab,
    ...restQuery
  } = currentQuery
  const query: LocationQueryRaw = { ...restQuery, view: destination.view }
  // A scope row names the configuration layer to open; every other destination
  // drops the scope so a page without a layer choice cannot inherit a stale one.
  if (destination.scope) query.scope = destination.scope
  if (
    destination.section === 'plugins-tools' &&
    pluginTabForSettingsView(destination.view) &&
    currentSection === 'plugins-tools' &&
    pluginTabForSettingsView(currentQuery.view)
  ) {
    if (plugin !== undefined) query.plugin = plugin
    const tab = pluginTabForSettingsView(currentQuery.view, currentQuery.pluginTab)
    if (tab && tab !== 'overview') query.pluginTab = tab
  }
  return query
}
