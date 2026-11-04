import { describe, expect, test } from 'bun:test'
import { pluginTabForSettingsView } from '../src/components/settings/pluginSettingsNavigation'
import { settingsQueryForDestination } from '../src/components/settings/settingsDestination'
import { buildSettingsSubpages } from '../src/components/settings/settingsNavigationCatalog'
import { buildSettingsScopes } from '../src/components/settings/settingsScopeCatalog'

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

test('runtime settings expose a scoped saved prompt library beside prompt attachments', () => {
  const pages = buildSettingsSubpages('runtime-session').map((page) => page.id)
  expect(pages).toContain('prompt-library')
  expect(pages).toContain('prompt-layers')
  expect(buildSettingsScopes('runtime-session', 'prompt-library').map((scope) => scope.id)).toEqual([
    'global',
    'workspace',
  ])
  expect(buildSettingsScopes('runtime-session', 'prompt-layers').map((scope) => scope.id)).toEqual([
    'effective',
    'global',
    'workspace',
    'session',
  ])
})
