import { describe, expect, test } from 'bun:test'
import { pluginTabForSettingsView } from '../src/components/settings/pluginSettingsNavigation'
import { movedSettingsPage, settingsQueryForDestination } from '../src/components/settings/settingsDestination'
import { buildSettingsSubpages, SETTINGS_DEFAULT_SUBPAGE } from '../src/components/settings/settingsNavigationCatalog'
import { buildSettingsScopes } from '../src/components/settings/settingsScopeCatalog'
import { resolveSettingsSubpage } from '../src/components/settings/workbench/settingsSectionNavigation'
import {
  legacyPermissionAccessScope,
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
  expect(legacyPermissionAccessScope('sandbox', undefined)).toBe('global')
  expect(legacyPermissionAccessScope('sandbox', 'workspace')).toBeUndefined()
  expect(legacyPermissionAccessScope('overview', 'global')).toBe('effective')
  expect(permissionAccessHash('sandbox')).toBe('#permission-sandbox')
  expect(
    settingsQueryForDestination({ section: 'permissions', view: 'access', scope: 'workspace' }, 'permissions', {}),
  ).toEqual({ view: 'access', scope: 'workspace' })
})

test('access is the default editor and old permission pages resolve without losing their scope', () => {
  const pages = buildSettingsSubpages('permissions')
  const ids = pages.map((page) => page.id)
  expect(ids).toEqual(['profiles', 'assignments', 'access'])
  expect(SETTINGS_DEFAULT_SUBPAGE.permissions).toBe('access')
  for (const oldView of ['filesystem', 'network', 'tools', 'sandbox', 'overview']) {
    expect(resolveSettingsSubpage(oldView, '', pages, 'access')).toBe('access')
    expect(resolveSettingsSubpage('', oldView, pages, 'access')).toBe('access')
    expect(buildSettingsScopes('permissions', oldView).map((scope) => scope.id)).toEqual([
      'effective',
      'global',
      'workspace',
      'session',
    ])
    expect(permissionSectionFromHash(permissionAccessHash(oldView))).toBe(oldView === 'overview' ? undefined : oldView)
  }
  expect(
    settingsQueryForDestination({ section: 'permissions', view: 'access', scope: 'session' }, 'permissions', {
      sessionId: '999',
      view: 'tools',
    }),
  ).toEqual({ view: 'access', scope: 'session' })
  expect(permissionSectionFromHash('#unrelated')).toBeUndefined()
})

test('prompt library bookmarks open the dedicated editor with two editable scopes', () => {
  const pages = buildSettingsSubpages('runtime-session')
  expect(pages.map((page) => page.id)).toEqual([
    'storage',
    'agent-profiles',
    'system-prompt',
    'prompt-library',
    'acp-agents',
    'client-versions',
    'session-behavior',
  ])
  expect(buildSettingsScopes('runtime-session', 'acp-agents').map((scope) => scope.id)).toEqual(['global'])
  expect(resolveSettingsSubpage('prompt-library', '', pages, 'session-behavior')).toBe('prompt-library')
  expect(resolveSettingsSubpage('', 'prompt-library', pages, 'session-behavior')).toBe('prompt-library')
  expect(resolveSettingsSubpage('prompt-layers', '', pages, 'session-behavior')).toBe('system-prompt')
  expect(buildSettingsScopes('runtime-session', 'system-prompt').map((scope) => scope.id)).toEqual([
    'global',
    'workspace',
    'session',
  ])
  expect(movedSettingsPage('runtime-session', 'agent-profiles', '#settings-prompt-library')).toEqual({
    section: 'runtime-session',
    view: 'prompt-library',
  })
  expect(buildSettingsScopes('runtime-session', 'prompt-library').map((scope) => scope.id)).toEqual([
    'global',
    'workspace',
  ])
})

test('Session settings owns the model scopes and legacy model and default profile links', () => {
  const pages = buildSettingsSubpages('models-providers')
  expect(pages.map((page) => page.id)).toEqual(['provider-studio', 'model-catalog'])
  for (const view of ['model-settings', 'defaults', 'approval-model']) {
    expect(movedSettingsPage('models-providers', view)).toEqual({
      section: 'runtime-session',
      view: 'session-behavior',
    })
  }
  expect(movedSettingsPage('models-providers', 'model-catalog')).toBeUndefined()
  expect(movedSettingsPage('models-providers', 'provider-studio')).toBeUndefined()
  expect(movedSettingsPage('runtime-session', 'agent-profiles')).toBeUndefined()
  expect(movedSettingsPage('runtime-session', 'agent-profiles', '#default-agent-profiles')).toEqual({
    section: 'runtime-session',
    view: 'session-behavior',
  })
  const runtime = buildSettingsSubpages('runtime-session')
  expect(resolveSettingsSubpage('compaction', '', runtime, 'session-behavior')).toBe('session-behavior')
  expect(SETTINGS_DEFAULT_SUBPAGE['runtime-session']).toBe('session-behavior')
  expect(buildSettingsScopes('runtime-session', 'session-behavior').map((scope) => scope.id)).toEqual([
    'global',
    'workspace',
    'effective',
    'session',
  ])
})

test('Web conversation and notification bookmarks open Web preferences', () => {
  const pages = buildSettingsSubpages('interface')
  expect(pages.map((page) => page.id)).toEqual(['tui', 'web-appearance'])
  for (const view of ['conversation', 'notifications']) {
    expect(resolveSettingsSubpage(view, '', pages, 'web-appearance')).toBe('web-appearance')
  }
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
      { section: 'runtime-session', view: 'session-behavior', scope: 'session' },
      'permissions',
      current,
    ),
  ).toEqual({ view: 'session-behavior', scope: 'session', targetSessionId: '42', windowId: 'settings-2' })
})
