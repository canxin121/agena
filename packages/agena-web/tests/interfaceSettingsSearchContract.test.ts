import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import test from 'node:test'

test('TUI interface settings provide searchable activity-kind and exact-tool overrides', () => {
  const source = readFileSync(resolve(import.meta.dir, '../src/components/settings/InterfaceSettingsPanel.vue'), 'utf8')
  assert.ok(source.includes('filteredActivityKinds'))
  assert.ok(source.includes('filteredToolNames'))
  assert.ok(source.includes('Filter activity kinds'))
  assert.ok(source.includes('Filter exact tools'))
  assert.ok(source.includes('v-for="name in filteredToolNames"'))
})
