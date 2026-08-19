import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import test from 'node:test'

test('server-backed TUI locale does not overwrite the browser-only Web locale', () => {
  const source = readFileSync(resolve(import.meta.dir, '../src/components/settings/InterfaceSettingsPanel.vue'), 'utf8')
  assert.ok(source.includes('path="ui.locale"'))
  assert.ok(source.includes('Web interface language is configured separately'))
  assert.ok(!source.includes('setAppLocale'))
  assert.ok(!source.includes('@saved='))
})
