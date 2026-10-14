import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import test from 'node:test'
import { settingsQueryForDestination } from '../src/components/settings/settingsDestination'
import {
  normalizePluginSettingsView,
  pluginTabForSettingsView,
  PLUGIN_SETTINGS_VIEWS,
} from '../src/components/settings/pluginSettingsNavigation'

test('settings navigation preserves workspace scope and clears unrelated deep-link state', () => {
  const current = {
    view: 'plugin-workbench',
    plugin: 'fs',
    pluginTab: 'logs',
    workspaceId: '42',
    windowId: 'settings',
    paneId: 'left',
  }
  assert.deepEqual(
    settingsQueryForDestination({ section: 'interface', view: 'conversation' }, 'plugins-tools', current),
    {
      view: 'conversation',
      workspaceId: '42',
      windowId: 'settings',
      paneId: 'left',
    },
  )
  assert.deepEqual(
    settingsQueryForDestination({ section: 'plugins-tools', view: 'plugin-logs' }, 'plugins-tools', current),
    {
      view: 'plugin-workbench',
      pluginTab: 'logs',
      plugin: 'fs',
      workspaceId: '42',
      windowId: 'settings',
      paneId: 'left',
    },
  )
  assert.equal(
    settingsQueryForDestination({ section: 'plugins-tools', view: 'mcp-server' }, 'plugins-tools', current).plugin,
    undefined,
  )
  assert.equal(
    settingsQueryForDestination({ section: 'plugins-tools', view: 'plugin-tools' }, 'interface', current).plugin,
    undefined,
  )
  // A scope row names the configuration layer to open; any other destination
  // drops it again, so a page without a layer choice cannot inherit one.
  assert.deepEqual(
    settingsQueryForDestination(
      { section: 'permissions', view: 'filesystem', scope: 'workspace' },
      'permissions',
      current,
    ),
    { view: 'filesystem', scope: 'workspace', workspaceId: '42', windowId: 'settings', paneId: 'left' },
  )
  assert.equal(
    settingsQueryForDestination({ section: 'interface', view: 'conversation' }, 'permissions', {
      ...current,
      view: 'filesystem',
      scope: 'workspace',
    }).scope,
    undefined,
  )
  assert.equal(
    settingsQueryForDestination({ section: 'diagnostics', view: 'advanced-settings', scope: 'global' }, 'permissions', {
      view: 'filesystem',
      scope: 'session',
    }).scope,
    'global',
  )
})

test('old plugin sidebar links and current plugin tabs resolve to one workbench', () => {
  for (const [view, tab] of Object.entries(PLUGIN_SETTINGS_VIEWS)) {
    assert.equal(normalizePluginSettingsView(view), 'plugin-workbench')
    assert.equal(pluginTabForSettingsView(view), tab)
    assert.equal(pluginTabForSettingsView('plugin-workbench', tab), tab)
  }
  assert.equal(pluginTabForSettingsView('plugin-workbench', 'untrusted-tab'), 'overview')
  assert.equal(pluginTabForSettingsView('plugin-workbench', 'constructor'), 'overview')
  assert.equal(pluginTabForSettingsView('plugin-logs', 'settings'), 'logs')
  assert.equal(normalizePluginSettingsView('marketplace'), 'marketplace')
  assert.equal(normalizePluginSettingsView(' PLUGIN-SETTINGS '), 'plugin-workbench')
  assert.equal(pluginTabForSettingsView('untrusted-tab'), null)
  assert.equal(pluginTabForSettingsView('constructor'), null)
  assert.equal(pluginTabForSettingsView('marketplace'), null)
})

test('returning to the installed plugin page preserves its selection and tab, while other pages clear both', () => {
  const query = { view: 'plugin-workbench', plugin: 'agena.web', pluginTab: 'settings', workspaceId: '42' }
  assert.deepEqual(
    settingsQueryForDestination({ section: 'plugins-tools', view: 'plugin-workbench' }, 'plugins-tools', query),
    query,
  )
  for (const view of ['marketplace', 'mcp-server']) {
    assert.deepEqual(settingsQueryForDestination({ section: 'plugins-tools', view }, 'plugins-tools', query), {
      view,
      workspaceId: '42',
    })
  }
  assert.deepEqual(
    settingsQueryForDestination({ section: 'plugins-tools', view: 'plugin-workbench' }, 'interface', query),
    { view: 'plugin-workbench', workspaceId: '42' },
  )
})

test('settings sidebar and content share one normalized routing state', () => {
  const source = readFileSync(resolve(import.meta.dir, '../src/pages/SettingsPage.vue'), 'utf8')
  assert.ok(source.includes('settingsQueryForDestination(destination, activeSection.value, route.query)'))
  assert.ok(source.includes('router.push({ path, query, hash: route.hash })'))
  assert.ok(source.includes(':active-view="activeView"'))
  assert.ok(source.includes(':active-scope="activeScope"'))
  assert.ok(source.includes(':active-page="activeView"'))
  assert.ok(source.includes('normalizePluginSettingsView('))
})
