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
      view: 'plugin-logs',
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
})

test('old plugin tab links normalize to matching second-level destinations', () => {
  for (const [view, tab] of Object.entries(PLUGIN_SETTINGS_VIEWS)) {
    assert.equal(normalizePluginSettingsView('plugin-workbench', tab), view)
    assert.equal(pluginTabForSettingsView(view), tab)
    assert.equal(normalizePluginSettingsView(view, 'invalid'), view)
  }
  assert.equal(normalizePluginSettingsView('plugin-workbench', 'untrusted-tab'), 'plugin-workbench')
  assert.equal(pluginTabForSettingsView('untrusted-tab'), null)
  assert.equal(pluginTabForSettingsView('constructor'), null)
  assert.equal(pluginTabForSettingsView('marketplace'), null)
})

test('settings sidebar and content share one normalized routing state', () => {
  const source = readFileSync(resolve(import.meta.dir, '../src/pages/SettingsPage.vue'), 'utf8')
  assert.ok(source.includes('settingsQueryForDestination(destination, activeSection.value, route.query)'))
  assert.ok(source.includes('router.push({ path, query, hash: route.hash })'))
  assert.ok(source.includes(':active-view="activeView"'))
  assert.ok(source.includes(':active-page="activeView"'))
  assert.ok(source.includes('normalizePluginSettingsView('))
})
