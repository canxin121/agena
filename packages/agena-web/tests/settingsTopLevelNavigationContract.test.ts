import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import test from 'node:test'

test('left settings navigation writes the selected subpage and clears unrelated deep-link state', () => {
  const source = readFileSync(resolve(import.meta.dir, '../src/pages/SettingsPage.vue'), 'utf8')
  assert.ok(source.includes('function goToSettingsDestination'))
  assert.ok(source.includes('view: _view'))
  assert.ok(source.includes('plugin, pluginTab'))
  assert.ok(source.includes('view: destination.view'))
  assert.ok(source.includes("destination.view === 'plugin-workbench'"))
  assert.ok(source.includes('router.push({ path, query, hash: route.hash })'))
  assert.ok(source.includes(':active-view="String(route.query.view'))
  assert.ok(source.includes('@navigate="goToSettingsDestination"'))
})
