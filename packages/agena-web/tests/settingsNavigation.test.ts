import { describe, expect, test } from 'bun:test'
import { pluginTabForSettingsView } from '../src/components/settings/pluginSettingsNavigation'
import { settingsQueryForDestination } from '../src/components/settings/settingsDestination'
import { buildSettingsSubpages, SETTINGS_DEFAULT_SUBPAGE } from '../src/components/settings/settingsNavigationCatalog'
import { buildSettingsScopes } from '../src/components/settings/settingsScopeCatalog'
import { resolveSettingsSubpage } from '../src/components/settings/workbench/settingsSectionNavigation'
import {
  permissionAccessHash,
  permissionSectionFromHash,
} from '../src/components/settings/permissions/permissionNavigation'

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

test('sandbox switches are reachable in permissions with global and workspace scopes', () => {
  expect(buildSettingsSubpages('permissions').map((page) => page.id)).toContain('sandbox')
  expect(buildSettingsScopes('permissions', 'sandbox').map((scope) => scope.id)).toEqual(['global', 'workspace'])
  expect(
    settingsQueryForDestination({ section: 'permissions', view: 'sandbox', scope: 'workspace' }, 'permissions', {}),
  ).toEqual({ view: 'sandbox', scope: 'workspace' })
})

test('access is the default editor and old permission pages resolve without losing their scope', () => {
  const pages = buildSettingsSubpages('permissions')
  const ids = pages.map((page) => page.id)
  expect(ids.filter((id) => id === 'access')).toEqual(['access'])
  expect(ids).toEqual(expect.arrayContaining(['profiles', 'sandbox', 'overview']))
  expect(ids.filter((id) => ['filesystem', 'network', 'tools'].includes(id))).toEqual([])
  expect(SETTINGS_DEFAULT_SUBPAGE.permissions).toBe('access')
  for (const oldView of ['filesystem', 'network', 'tools']) {
    expect(resolveSettingsSubpage(oldView, '', pages, 'access')).toBe('access')
    expect(resolveSettingsSubpage('', oldView, pages, 'access')).toBe('access')
    expect(buildSettingsScopes('permissions', oldView).map((scope) => scope.id)).toEqual([
      'effective',
      'global',
      'workspace',
      'session',
    ])
    expect(permissionSectionFromHash(permissionAccessHash(oldView))).toBe(oldView)
  }
  expect(
    settingsQueryForDestination({ section: 'permissions', view: 'access', scope: 'session' }, 'permissions', {
      sessionId: '999',
      view: 'tools',
    }),
  ).toEqual({ view: 'access', scope: 'session' })
  expect(permissionSectionFromHash('#unrelated')).toBeUndefined()
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

test('settings destinations preserve the explicit target and pane without carrying the retired chat query', () => {
  const current = { view: 'access', scope: 'session', targetSessionId: '42', sessionId: '999', windowId: 'settings-2' }
  expect(
    settingsQueryForDestination(
      { section: 'permissions', view: 'overview', scope: 'effective' },
      'permissions',
      current,
    ),
  ).toEqual({ view: 'overview', scope: 'effective', targetSessionId: '42', windowId: 'settings-2' })
  expect(
    settingsQueryForDestination(
      { section: 'models-providers', view: 'model-settings', scope: 'session' },
      'permissions',
      current,
    ),
  ).toEqual({ view: 'model-settings', scope: 'session', targetSessionId: '42', windowId: 'settings-2' })
})
