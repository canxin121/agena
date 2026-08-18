import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import test from 'node:test'

test('top-level settings navigation clears stale subpage and plugin deep-link keys', () => {
  const source = readFileSync(resolve(import.meta.dir, '../src/pages/SettingsPage.vue'), 'utf8')
  assert.ok(source.includes('view: _view'))
  assert.ok(source.includes('plugin: _plugin'))
  assert.ok(source.includes('pluginTab: _pluginTab'))
  assert.ok(source.includes('router.push({ path, query, hash: route.hash })'))
})
