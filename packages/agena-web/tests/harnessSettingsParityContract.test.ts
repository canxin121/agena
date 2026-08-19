import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import test from 'node:test'

test('Harness settings expose every runtime field and explicit Global/Workspace layers', () => {
  const source = readFileSync(resolve(import.meta.dir, '../src/components/settings/HarnessSettingsPanel.vue'), 'utf8')
  for (const value of [
    "targetLayer = ref<RuntimeSettingsLayer>('global')",
    'Copy effective catalog',
    'launch_options',
    'allowed_domains',
    'allow_commands',
    'deny_commands',
    'current.env',
    'max_file_bytes',
    'allowed_extensions',
    'renameHarness',
  ]) {
    assert.ok(source.includes(value), `missing Harness capability ${value}`)
  }
  assert.ok(source.includes('setRuntimeSetting('))
  assert.ok(source.includes('targetLayer.value'))
})
