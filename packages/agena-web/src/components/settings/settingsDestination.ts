import type { LocationQuery, LocationQueryRaw } from 'vue-router'
import type { SettingsSidebarDestination, SettingsTab } from './sidebar/settingsSidebarNavigation'
import { pluginTabForSettingsView } from './pluginSettingsNavigation'
import { normalizeSettingsSubpageId } from './workbench/settingsSectionNavigation'

export const AGENT_PROFILE_DEFAULTS_HASH = '#settings-default-agent-profiles'
export const PROMPT_LIBRARY_HASH = '#settings-prompt-library'

export function isAgentProfileDefaultsHash(hash: string): boolean {
  return hash === AGENT_PROFILE_DEFAULTS_HASH || hash === '#default-agent-profiles'
}

export function movedSettingsPage(
  section: SettingsTab,
  view: unknown,
  hash = '',
): SettingsSidebarDestination | undefined {
  const id = normalizeSettingsSubpageId(view)
  if (section === 'runtime-session' && hash === PROMPT_LIBRARY_HASH) {
    return { section: 'runtime-session', view: 'prompt-library' }
  }
  if (
    (section === 'models-providers' && ['model-settings', 'defaults', 'approval-model'].includes(id)) ||
    (section === 'runtime-session' && id === 'agent-profiles' && isAgentProfileDefaultsHash(hash))
  ) {
    return { section: 'runtime-session', view: 'session-behavior' }
  }
  return undefined
}

export function agentProfileDefaultsDestination(scope: 'global' | 'workspace', query: LocationQuery) {
  return {
    path: '/settings/runtime-session',
    query: settingsQueryForDestination(
      { section: 'runtime-session', view: 'session-behavior', scope },
      'runtime-session',
      query,
    ),
    hash: AGENT_PROFILE_DEFAULTS_HASH,
  }
}

export function settingsQueryForDestination(
  destination: SettingsSidebarDestination,
  currentSection: SettingsTab,
  currentQuery: LocationQuery,
): LocationQueryRaw {
  // The retired chat `sessionId` is not a settings parameter. An explicitly
  // selected `targetSessionId` stays in restQuery across settings destinations.
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
