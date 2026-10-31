import { describe, expect, test } from 'bun:test'
import { pluginTabForSettingsView } from '../src/components/settings/pluginSettingsNavigation'
import { settingsQueryForDestination } from '../src/components/settings/settingsDestination'

describe('current plugin settings route contract', () => {
  test('workbench tabs have one route and retired sidebar views are not upgraded', () => {
    expect(pluginTabForSettingsView('plugin-workbench', 'settings')).toBe('settings')
    expect(pluginTabForSettingsView('plugin-workbench', 'unknown')).toBe('overview')
    for (const view of ['plugin-settings', 'plugin-commands', 'plugin-tools', 'plugin-logs', 'plugin-diagnostics']) {
      expect(pluginTabForSettingsView(view)).toBeNull()
    }
  })

  test('navigation retains the current plugin tab only inside its workbench', () => {
    const current = { view: 'plugin-workbench', plugin: 'fixture', pluginTab: 'settings', scope: 'session' }
    expect(
      settingsQueryForDestination({ section: 'plugins-tools', view: 'plugin-workbench' }, 'plugins-tools', current),
    ).toEqual({ view: 'plugin-workbench', plugin: 'fixture', pluginTab: 'settings' })
    expect(
      settingsQueryForDestination({ section: 'plugins-tools', view: 'marketplace' }, 'plugins-tools', current),
    ).toEqual({ view: 'marketplace' })
    expect(
      settingsQueryForDestination({ section: 'permissions', view: 'rules', scope: 'global' }, 'plugins-tools', current),
    ).toEqual({ view: 'rules', scope: 'global' })
  })
})
