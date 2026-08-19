import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import test from 'node:test'

test('plugin config editor exposes structured schema editing, dry-run validation, diff, and reload save', () => {
  const source = readFileSync(
    resolve(import.meta.dir, '../src/components/settings/plugins/PluginConfigEditor.vue'),
    'utf8',
  )
  assert.ok(source.includes('<JsonSchemaField'))
  assert.ok(source.includes('{ dry_run: true, validate: true, reload: false }'))
  assert.ok(source.includes('{ validate: true, reload: true }'))
  assert.ok(source.includes('Configuration diff & persisted override'))
  assert.ok(source.includes('deriveConfigOverride'))
  assert.ok(source.includes('Plugin enabled'))
})
